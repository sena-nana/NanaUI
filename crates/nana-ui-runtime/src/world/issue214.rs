//! Issue #214: the structural gates of Dynamic Layout. Counters, not time.
//!
//! - A: a static frame and a paint-only change run no solve and ask no
//!   envelope; a container that does not solve never reads a declaration.
//! - B: a thousand elastic items short by what their padding covers: one
//!   segment read per item, nothing deeper, linear from 1k to 2k, and a
//!   resize inside the level allocates nothing.
//! - C: a nested elastic tree, four children a level: every node resolves
//!   at most once a pass and its envelope is built once; work grows with
//!   the nodes, not with branching to the depth.
//! - E: 240 resizes across three levels: after warm-up no envelope is built,
//!   and a solve runs from scratch only where the level changes.
//! - F: a line more crowded than the solver's budget falls back once per
//!   solve, the same way every time.
//! - G: 500 frames of resizes, item churn and profile edits leave the kept
//!   envelopes and solves what the tree gives them.
//! - D, the break-heavy text gate, is in `nana-text`'s `line_decision_gate`.

#![cfg(test)]

use nana_ui_core::dynamic_layout::{
    AdaptationProfile, AxisElasticity, ElasticFloor, ElasticLength, costs,
};
use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, SemanticColorRole, WorkCounters};

use super::issue212::{Bar, Toolbar, padding_profile};
use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, fixed, product_frame, styled,
};
use super::{DocumentId, StableNodeId};
use crate::layout_engine::measure_trace;
use crate::layout_engine::verify::skip_layout_verify;
use crate::{AppContext, LayoutViewport, MutationQueue, NanaTextEngineShaper};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(100_000.0, 400.0)
}

/// A row of `count` padded leaves, `width` wide, solving its overflow.
struct Row {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    row: StableNodeId,
    leaves: Vec<StableNodeId>,
    width: f32,
}

impl Row {
    fn new(count: usize, width: f32) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut b, root) = Builder::new(document, 1);
        let row = b.element(root, row_style(width));
        let leaves = (0..count)
            .map(|_| {
                let leaf = b.element(row, leaf_style());
                b.element(leaf, fixed(4.0, 4.0));
                leaf
            })
            .collect();
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut row = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            row,
            leaves,
            width,
        };
        row.frame();
        row
    }

    fn frame(&mut self) -> WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }

    fn resize(&mut self, width: f32) -> WorkCounters {
        self.width = width;
        let mut queue = MutationQueue::new();
        queue.set_style(self.row, styled(row_style(width)));
        self.context.commit_mutations(queue).unwrap();
        self.frame()
    }
}

fn row_style(width: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(20.0)),
        direction: Some(FlexDirection::Row),
        adaptation: Some(AdaptationProfile {
            solve_overflow: true,
            ..AdaptationProfile::RIGID
        }),
        ..LayoutStyle::default()
    }
}

/// 36 px preferred (16 + 4 + 16), 24 px of it padding that may close up.
fn leaf_style() -> LayoutStyle {
    LayoutStyle {
        direction: Some(FlexDirection::Row),
        padding_left: Some(LengthSpec::Px(16.0)),
        padding_right: Some(LengthSpec::Px(16.0)),
        adaptation: Some(padding_profile()),
        ..LayoutStyle::default()
    }
}

/// Gate A.
#[test]
fn issue214_a_static_frame_and_a_paint_change_solve_nothing() {
    let mut toolbar = Toolbar::new(Bar::new(330.0));
    let first = toolbar.context.last_work_counters().dynamic;
    assert!(first.solver_runs > 0);
    let applied_before = toolbar.item_boxes();
    // A static frame has no work at all, so no layout and no solve.
    assert!(toolbar.context.take_system_work().is_empty());
    // A paint-only edit of every item.
    let mut queue = MutationQueue::new();
    for item in &toolbar.items {
        let mut style = toolbar.context.world().node_style(*item).unwrap().clone();
        style.background = Some(SemanticColorRole::Accent);
        queue.set_style(*item, style);
    }
    toolbar.context.commit_mutations(queue).unwrap();
    // Paint reaches no layout: no seed, so no solve, no envelope asked for.
    let work = toolbar.context.take_system_work();
    assert!(
        work.layout_frontier_seeds.is_empty(),
        "{:?}",
        work.layout_frontier_seeds
    );
    assert!(
        toolbar
            .context
            .take_layout_frontier_seeds(toolbar.document)
            .is_empty()
    );
    assert_eq!(toolbar.item_boxes(), applied_before);
}

/// Gate B.
#[test]
fn issue214_b_a_cheap_deficit_reads_each_item_once() {
    let _unguarded = skip_layout_verify();
    let mut costs = Vec::new();
    for count in [1_000usize, 2_000] {
        // Short by half of what the padding covers.
        let mut row = Row::new(count, count as f32 * 24.0);
        let first = row.context.last_work_counters().dynamic;
        assert_eq!(first.cost_segments_visited, count, "{count}");
        assert_eq!(
            (
                first.deep_expansions,
                first.child_reflows,
                first.budget_fallbacks
            ),
            (0, 0, 0)
        );
        assert_eq!(first.child_resolves, count);
        // A resize inside the level: re-shared, nothing allocated, and no
        // leaf's content measured again.
        measure_trace::begin();
        let next = row.resize(count as f32 * 24.0 + 7.0).dynamic;
        let measured = measure_trace::take();
        assert_eq!((next.cold_solves, next.allocations), (0, 0), "{next:?}");
        assert!(next.incremental_assignments > 0);
        for leaf in &row.leaves {
            let content = row.context.world().node(*leaf).unwrap().children[0];
            assert!(
                !measured.contains(&content),
                "{count}: leaf content measured"
            );
        }
        costs.push(first);
    }
    let ratio = |a: usize, b: usize| b as f64 / a as f64;
    assert!(
        ratio(
            costs[0].cost_segments_visited,
            costs[1].cost_segments_visited
        ) <= 2.05
    );
    assert!(ratio(costs[0].child_resolves, costs[1].child_resolves) <= 2.05);
}

/// An elastic tree, `branching` children a level, `depth` levels of rows
/// over padded leaves: every row passes its children's elasticity up and
/// solves its own line.
fn nested(depth: u32, branching: usize, width: f32) -> (AppContext, DocumentId, usize) {
    fn row(b: &mut Builder, parent: StableNodeId, depth: u32, branching: usize, nodes: &mut usize) {
        *nodes += 1;
        if depth == 0 {
            let leaf = b.element(
                parent,
                LayoutStyle {
                    direction: Some(FlexDirection::Row),
                    padding_left: Some(LengthSpec::Px(8.0)),
                    padding_right: Some(LengthSpec::Px(8.0)),
                    adaptation: Some(padding_profile()),
                    ..LayoutStyle::default()
                },
            );
            b.element(leaf, fixed(4.0, 4.0));
            return;
        }
        let elastic = AdaptationProfile {
            inline: AxisElasticity {
                padding: Some(ElasticLength::new(ElasticFloor::Px(1.0), costs::PADDING)),
                gap: Some(ElasticLength::new(
                    ElasticFloor::Px(1.0),
                    costs::PLACEMENT_GAP,
                )),
                content_gap: None,
            },
            aggregate_children: true,
            ..AdaptationProfile::RIGID
        };
        let node = b.element(
            parent,
            LayoutStyle {
                direction: Some(FlexDirection::Row),
                padding_left: Some(LengthSpec::Px(4.0)),
                padding_right: Some(LengthSpec::Px(4.0)),
                gap: Some(LengthSpec::Px(4.0)),
                adaptation: Some(elastic),
                ..LayoutStyle::default()
            },
        );
        for _ in 0..branching {
            row(b, node, depth - 1, branching, nodes);
        }
    }
    let document = DocumentId::new(1).unwrap();
    let (mut b, root) = Builder::new(document, 1);
    let top = b.element(root, row_style(width));
    let mut nodes = 0;
    for _ in 0..branching {
        row(&mut b, top, depth, branching, &mut nodes);
    }
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    (context, document, nodes)
}

/// Gate C.
#[test]
fn issue214_c_a_nested_tree_resolves_each_node_once() {
    let mut work = Vec::new();
    for depth in [2u32, 3] {
        let (mut context, document, elastic_nodes) = nested(depth, 4, 200.0);
        let mut shaper = bundled_face_shaper();
        let first = product_frame(&mut context, document, viewport(), &mut shaper).dynamic;
        assert!(first.child_resolves > 0);
        assert!(first.child_resolves <= elastic_nodes, "{depth}: {first:?}");
        assert!(first.envelope_misses <= elastic_nodes, "{depth}: {first:?}");
        // Resize the top row a little: nothing is built again.
        let mut queue = MutationQueue::new();
        let top = context
            .world()
            .node(StableNodeId::new(1).unwrap())
            .unwrap()
            .children[0];
        queue.set_style(top, styled(row_style(203.0)));
        context.commit_mutations(queue).unwrap();
        let next = product_frame(&mut context, document, viewport(), &mut shaper).dynamic;
        assert_eq!(
            next.envelope_misses + next.envelope_rebuilds,
            0,
            "{depth}: {next:?}"
        );
        assert!(next.child_resolves <= elastic_nodes);
        let mut cold = nested(depth, 4, 203.0).0;
        product_frame(&mut cold, document, viewport(), &mut bundled_face_shaper());
        assert_matches_cold(&mut context, &mut cold, document);
        work.push((
            elastic_nodes,
            first.child_resolves + first.cost_segments_visited,
        ));
    }
    let node_ratio = work[1].0 as f64 / work[0].0 as f64;
    let work_ratio = work[1].1 as f64 / work[0].1 as f64;
    assert!(work_ratio <= node_ratio * 1.05, "{work:?}");
}

/// Gate E.
#[test]
fn issue214_e_a_resize_storm_builds_nothing_and_solves_cold_only_across_levels() {
    // The toolbar's levels: padding below 442 px, gaps below 472 px.
    let level = |width: f32| {
        if width >= 472.0 {
            0
        } else if width >= 442.0 {
            1
        } else {
            2
        }
    };
    let mut toolbar = Toolbar::new(Bar::new(301.0));
    let widths: Vec<f32> = (0..240)
        .map(|frame| {
            let step = frame % 120;
            let out = if step < 60 { step } else { 120 - step };
            300.0 + out as f32 * 3.0
        })
        .collect();
    let mut previous = level(300.0);
    for (frame, width) in widths.iter().enumerate() {
        let dynamic = toolbar.resize(*width).dynamic;
        assert_eq!(
            dynamic.envelope_misses + dynamic.envelope_rebuilds,
            0,
            "{width}"
        );
        let now = level(*width);
        let expected_cold = usize::from(now != 0 && now != previous);
        assert_eq!(
            dynamic.cold_solves, expected_cold,
            "{frame} {width}: {dynamic:?}"
        );
        previous = now;
        if frame % 16 == 0 {
            let mut cold = Toolbar::new(Bar::new(*width));
            assert_matches_cold(&mut toolbar.context, &mut cold.context, toolbar.document);
        }
    }
}

/// Gate F.
#[test]
fn issue214_f_a_line_past_the_budget_falls_back_the_same_way_every_time() {
    let _unguarded = skip_layout_verify();
    let count = crate::layout_engine::dynamic_line_max_states() + 904;
    let width = count as f32 * 30.0;
    let mut runs = Vec::new();
    for _ in 0..2 {
        let row = Row::new(count, width);
        let dynamic = row.context.last_work_counters().dynamic;
        assert_eq!(dynamic.budget_fallbacks, 1, "{dynamic:?}");
        assert!(dynamic.cost_segments_visited <= count);
        runs.push(
            row.leaves
                .iter()
                .map(|leaf| row.context.world().layout_box(*leaf).unwrap())
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(runs[0], runs[1]);
    let mut row = Row::new(count, width);
    let mut cold = Row::new(count, width);
    assert_matches_cold(&mut row.context, &mut cold.context, row.document);
}

/// Gate G.
#[test]
fn issue214_g_kept_envelopes_and_solves_follow_the_tree() {
    let mut toolbar = Toolbar::new(Bar::new(330.0));
    let spare = toolbar.items[5];
    let mut at_ten = None;
    for frame in 0..500u32 {
        let width = 300.0 + (frame % 40) as f32 * 4.0;
        if frame % 50 == 25 {
            let mut queue = MutationQueue::new();
            queue.detach(spare);
            toolbar.context.commit_mutations(queue).unwrap();
            toolbar.frame();
            let mut queue = MutationQueue::new();
            queue.insert(toolbar.bar, spare, None);
            toolbar.context.commit_mutations(queue).unwrap();
        }
        if frame % 100 == 50 {
            let mut style = toolbar.context.world().node_style(spare).unwrap().clone();
            let mut profile = padding_profile();
            profile.inline.padding = Some(ElasticLength::new(
                ElasticFloor::Px(2.0 + (frame / 100) as f32),
                costs::PADDING,
            ));
            std::sync::Arc::make_mut(&mut style.layout).adaptation = Some(profile);
            let mut queue = MutationQueue::new();
            queue.set_style(spare, style);
            toolbar.context.commit_mutations(queue).unwrap();
        }
        toolbar.resize(width);
        let footprint = toolbar.context.retained_layout_footprint(toolbar.document);
        let nodes = toolbar.context.world().len();
        assert!(footprint.envelopes <= nodes * 2);
        assert!(footprint.line_solve_entries <= toolbar.items.len() * footprint.line_solves);
        if frame == 10 {
            at_ten = Some(footprint);
        }
        if frame == 490 {
            let at_ten = at_ten.unwrap();
            assert_eq!(
                (
                    footprint.envelopes,
                    footprint.line_solves,
                    footprint.applied_adjustments
                ),
                (
                    at_ten.envelopes,
                    at_ten.line_solves,
                    at_ten.applied_adjustments
                )
            );
        }
    }
}
