//! Issue #264: a resize reaches the boxes that consume the constraint that
//! changed, and nothing else.
//!
//! One workspace: a fixed dock on the left -- the filler that takes the
//! document from 1k to 100k nodes -- and a real [`SplitPane`] beside it. Its
//! first pane holds a fill-width toolbar of fixed buttons, a list of
//! fill-width rows, a wrapping paragraph, a fixed box with a subtree of its
//! own and a box whose height is half the pane's; the second holds a
//! toolbar, a paragraph and fixed cards. The splitter drags through
//! `SplitPaneMutation::SetSize`, which writes the first pane's width.
//!
//! - Gate A: 240 drags in a 10k workspace. No fixed box, and no box of the
//!   dock, is measured; no text shapes again; the nodes are the same at the
//!   end; every frame matches a cold layout.
//! - Gate B: a width-only change never measures the box that reads only the
//!   pane's height, and the paragraph rewraps from its shaped runs; a
//!   height-only change never measures the fill-width rows.
//! - Gate C: the fixed box and everything under it never measure.
//! - Gate D: a drag costs the same in a 1k and a 100k workspace, and asks
//!   the same children whether they consume the constraint.
//! - A viewport resize reaches the roots and the boxes that consume the
//!   axis it moved on, and costs the same at 1k and 100k nodes.
//!
//! Gate E -- an available extent that stays inside a cached envelope -- needs
//! the Dynamic Layout solver of #207 and #213, which the runtime does not
//! have yet.

#![cfg(test)]

use nana_ui_core::{
    AlignSpec, FlexDirection, LayoutStyle, LengthSpec, SplitAxis, SplitPaneModel,
    SplitPaneMutation, WorkCounters,
};

use super::reflow_oracle::{
    Builder, admitted, assert_matches_cold, bundled_face_shaper, fixed, measured, product_frame,
    resize_frame, styled,
};
use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::layout_engine::verify::skip_layout_verify;
use crate::{AppContext, Entity, LayoutViewport, MutationQueue, NanaTextEngineShaper, SplitPane};

const PARAGRAPH: &str = "a paragraph long enough to wrap over a few lines of the pane it sits in";
const FIRST_SIZE: f32 = 300.0;

fn viewport() -> LayoutViewport {
    LayoutViewport::new(1200.0, 800.0)
}

fn fill_column() -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Fill),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    }
}

/// A pane of the split: it fills the split on both axes.
fn pane() -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Fill),
        height: Some(LengthSpec::Fill),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    }
}

/// A fill-width toolbar of ten fixed buttons.
fn toolbar(b: &mut Builder, parent: StableNodeId) -> Vec<StableNodeId> {
    let bar = b.element(
        parent,
        LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Px(32.0)),
            direction: Some(FlexDirection::Row),
            ..LayoutStyle::default()
        },
    );
    (0..10)
        .map(|_| {
            let button = b.element(bar, fixed(28.0, 28.0));
            b.label(button, "B");
            button
        })
        .collect()
}

/// Nodes the gates name.
struct Parts {
    dock: StableNodeId,
    /// Fixed boxes of both panes: toolbar buttons and cards.
    fixed: Vec<StableNodeId>,
    /// A fixed box with a subtree of its own.
    fixed_box: StableNodeId,
    /// A box whose width is fixed and whose height is half the pane's.
    tall: StableNodeId,
    /// Fill-width rows of the first pane's list.
    rows: Vec<StableNodeId>,
    paragraph: StableNodeId,
}

struct Workspace {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    split: Entity<SplitPane>,
    parts: Parts,
    size: f32,
}

impl Workspace {
    /// A 1200x800 page: a dock of `dock_cards` fixed cards, and the split.
    fn new(dock_cards: u64, size: f32) -> Self {
        Self::build(dock_cards, size, false)
    }

    /// The same, on a page that fills the viewport.
    fn fluid(dock_cards: u64) -> Self {
        Self::build(dock_cards, FIRST_SIZE, true)
    }

    fn build(dock_cards: u64, size: f32, fluid: bool) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut b, root) = Builder::new(document, 1);
        let (width, height) = if fluid {
            (LengthSpec::Fill, LengthSpec::Fill)
        } else {
            (LengthSpec::Px(1200.0), LengthSpec::Px(800.0))
        };
        let page = b.element(
            root,
            LayoutStyle {
                width: Some(width),
                height: Some(height),
                direction: Some(FlexDirection::Row),
                align_items: AlignSpec::Stretch,
                ..LayoutStyle::default()
            },
        );
        let dock = b.element(
            page,
            LayoutStyle {
                width: Some(LengthSpec::Px(300.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            },
        );
        for _ in 0..dock_cards {
            let card = b.element(dock, fixed(280.0, 24.0));
            b.label(card, "dock");
        }
        let mut fixed_boxes = Vec::new();
        let first = b.detached(pane());
        fixed_boxes.extend(toolbar(&mut b, first));
        let list = b.element(first, fill_column());
        let rows = (0..40)
            .map(|index| {
                let row = b.element(list, fill_column());
                b.label(row, &format!("Item {index}"));
                row
            })
            .collect();
        let text_box = b.element(first, fill_column());
        let paragraph = b.label(text_box, PARAGRAPH);
        let fixed_box = b.element(
            first,
            LayoutStyle {
                direction: Some(FlexDirection::Column),
                ..fixed(200.0, 100.0)
            },
        );
        for _ in 0..4 {
            let row = b.element(fixed_box, fill_column());
            b.label(row, "inside");
        }
        let tall = b.element(
            first,
            LayoutStyle {
                width: Some(LengthSpec::Px(80.0)),
                height: Some(LengthSpec::Percent(50.0)),
                ..LayoutStyle::default()
            },
        );
        let second = b.detached(pane());
        fixed_boxes.extend(toolbar(&mut b, second));
        let text_box = b.element(second, fill_column());
        b.label(text_box, PARAGRAPH);
        for _ in 0..20 {
            let card = b.element(second, fixed(200.0, 40.0));
            b.label(card, "card");
            fixed_boxes.push(card);
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let handle = context
            .create_view(
                document,
                NodeKind::Element {
                    tag: "handle".into(),
                },
                (),
            )
            .unwrap()
            .stable_id();
        let mut model = SplitPaneModel::new(SplitAxis::Horizontal, FIRST_SIZE, 100.0, 800.0);
        model.update(SplitPaneMutation::SetSize(size));
        let split = context
            .create_component(
                document,
                SplitPane::from_model(&model, first, second).handle(handle),
            )
            .unwrap();
        context.assemble_split_pane(split).unwrap();
        let mut queue = MutationQueue::new();
        queue.insert(page, split.stable_id(), None);
        context.commit_mutations(queue).unwrap();
        let mut workspace = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            split,
            parts: Parts {
                dock,
                fixed: fixed_boxes,
                fixed_box,
                tall,
                rows,
                paragraph,
            },
            size,
        };
        workspace.frame();
        workspace
    }

    fn world(&self) -> &UiWorld {
        self.context.world()
    }

    fn frame(&mut self) -> WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }

    fn resize(&mut self, viewport: LayoutViewport) -> WorkCounters {
        resize_frame(&mut self.context, self.document, viewport, &mut self.shaper)
    }

    /// Drag the splitter to `size`, one frame.
    fn drag_to(&mut self, size: f32) -> WorkCounters {
        self.size = size;
        self.context
            .update_component(self.split, |pane, _| {
                pane.apply(SplitPaneMutation::SetSize(size))
            })
            .unwrap();
        self.frame()
    }

    fn subtree(&self, root: StableNodeId) -> Vec<StableNodeId> {
        let mut ids = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            ids.push(id);
            stack.extend(self.world().node(id).unwrap().children);
        }
        ids
    }
}

/// What a frame cost layout, in the units the gates name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cost {
    seeds: usize,
    frontier_measure: usize,
    frontier_placement: usize,
    contexts: usize,
    edges: usize,
    measured: usize,
    placed: usize,
    children_walked: usize,
    full_subtrees: usize,
    constraint_seeds: usize,
    considered: usize,
    remeasured: usize,
    skipped: usize,
    context_solves: usize,
    local_fallbacks: usize,
    full_fallbacks: usize,
}

impl From<WorkCounters> for Cost {
    fn from(counters: WorkCounters) -> Self {
        Self {
            seeds: counters.layout_frontier_seeds,
            frontier_measure: counters.layout_frontier_nodes_measure,
            frontier_placement: counters.layout_frontier_nodes_placement,
            contexts: counters.layout_frontier_contexts,
            edges: counters.layout_dependency_edges_visited,
            measured: counters.layout_measure_nodes,
            placed: counters.layout_placement_nodes,
            children_walked: counters.layout_children_measured,
            full_subtrees: counters.intrinsic_measure_full_subtrees,
            constraint_seeds: counters.constraint_change_seeds,
            considered: counters.constraint_dependents_considered,
            remeasured: counters.constraint_dependents_remeasured,
            skipped: counters.constraint_dependents_skipped,
            context_solves: counters.resize_context_solves,
            local_fallbacks: counters.layout_local_subtree_fallbacks,
            full_fallbacks: counters.layout_full_document_fallbacks,
        }
    }
}

/// The drags Gates A and B make: out by 120 pixels and back, one each frame.
fn drag_sizes() -> impl Iterator<Item = f32> {
    (1..=240).map(|step| {
        let out = if step <= 120 { step } else { 240 - step };
        FIRST_SIZE + out as f32
    })
}

/// Gates A, B and C over 240 drags of a 10k workspace.
#[test]
fn issue264_a_splitter_drag_lays_out_only_what_reads_the_pane_width() {
    // A full layout after each of 240 frames of 10k nodes would cost more
    // than the frames; the cold oracle checks the end, and a 1k workspace
    // runs the per-pass guard below.
    let _unguarded = skip_layout_verify();
    let mut workspace = Workspace::new(5_000, FIRST_SIZE);
    assert!(workspace.world().len() >= 10_000);
    let nodes = workspace.world().document_order(workspace.document);
    let dock = workspace.subtree(workspace.parts.dock);
    let fixed_subtree = workspace.subtree(workspace.parts.fixed_box);
    let mut relayouts = 0;
    let mut first: Option<Cost> = None;
    for size in drag_sizes() {
        let counters = workspace.drag_to(size);
        let cost = Cost::from(counters);
        assert_eq!(
            (cost.local_fallbacks, cost.full_fallbacks),
            (0, 0),
            "{size}: {cost:?}"
        );
        // Gate A: the text that rewraps rewraps from the runs it shaped.
        assert_eq!(
            counters.resize_text_reshapes, 0,
            "{size}: text shaped again"
        );
        relayouts += counters.resize_text_relayouts;
        // Gate A: the dock reads nothing of the split.
        for id in &dock {
            assert!(
                !admitted(&workspace.context, *id),
                "{size}: dock node {id:?} laid out"
            );
        }
        // Gates A and C: no fixed box measures, nor anything under the
        // fixed box.
        for id in workspace.parts.fixed.iter().chain(&fixed_subtree) {
            assert!(
                !measured(&workspace.context, *id),
                "{size}: fixed {id:?} measured"
            );
        }
        // Gate B: a width-only change; the box that reads only the pane's
        // height keeps its measurement.
        assert!(
            !measured(&workspace.context, workspace.parts.tall),
            "{size}: the height-only box measured"
        );
        // Each drag asks the same children the same question.
        match first {
            None => first = Some(cost),
            Some(first) => assert_eq!(
                (first.considered, first.remeasured, first.skipped),
                (cost.considered, cost.remeasured, cost.skipped),
                "{size}"
            ),
        }
    }
    assert!(relayouts > 0, "the paragraph never rewrapped");
    // Gate A: no node came or went.
    assert_eq!(
        workspace.world().document_order(workspace.document),
        nodes,
        "lifecycle churn"
    );
    let mut cold = Workspace::new(5_000, workspace.size);
    assert_matches_cold(
        &mut workspace.context,
        &mut cold.context,
        workspace.document,
    );
}

/// The same drags on a 1k workspace, each pass checked against a full
/// layout.
#[test]
fn issue264_splitter_drags_match_a_full_layout_every_pass() {
    let mut workspace = Workspace::new(100, FIRST_SIZE);
    for size in drag_sizes().step_by(7) {
        workspace.drag_to(size);
    }
    let mut cold = Workspace::new(100, workspace.size);
    assert_matches_cold(
        &mut workspace.context,
        &mut cold.context,
        workspace.document,
    );
}

/// Gate B the other way: a height-only change never measures the boxes
/// that read only the width, and measures the one that reads the height.
#[test]
fn issue264_a_height_only_resize_leaves_width_readers_alone() {
    let mut workspace = Workspace::fluid(100);
    let counters = workspace.resize(LayoutViewport::new(1200.0, 760.0));
    assert_eq!(counters.layout_full_document_fallbacks, 0);
    for row in &workspace.parts.rows {
        assert!(
            !measured(&workspace.context, *row),
            "fill-width row {row:?} measured"
        );
    }
    assert!(!measured(&workspace.context, workspace.parts.paragraph));
    assert_eq!(counters.resize_text_reshapes, 0);
    let mut cold = Workspace::fluid(100);
    cold.resize(LayoutViewport::new(1200.0, 760.0));
    assert_matches_cold(
        &mut workspace.context,
        &mut cold.context,
        workspace.document,
    );
}

/// Gate D. A drag costs the same in a workspace of 1k nodes as of 100k, and
/// asks the same children whether they consume the constraint.
#[test]
fn issue264_a_drag_costs_the_same_at_1k_and_100k_nodes() {
    let _unguarded = skip_layout_verify();
    let mut small = Workspace::new(400, FIRST_SIZE);
    let mut huge = Workspace::new(50_000, FIRST_SIZE);
    assert!(small.world().len() <= 1_200);
    assert!(huge.world().len() >= 100_000);
    for size in [FIRST_SIZE + 1.0, FIRST_SIZE + 40.0, FIRST_SIZE - 30.0] {
        let small_cost = Cost::from(small.drag_to(size));
        let huge_cost = Cost::from(huge.drag_to(size));
        assert_eq!(small_cost, huge_cost, "{size}");
        assert!(small_cost.constraint_seeds > 0);
    }
    let mut cold = Workspace::new(50_000, huge.size);
    assert_matches_cold(&mut huge.context, &mut cold.context, huge.document);
}

/// A viewport resize goes the same way: the roots, then the boxes that
/// consume the axis it moved on. The dock stays where it was and is not
/// laid out, and the cost is the same at 1k and 100k nodes.
#[test]
fn issue264_a_viewport_resize_reaches_only_its_consumers() {
    let _unguarded = skip_layout_verify();
    let mut small = Workspace::fluid(400);
    let mut huge = Workspace::fluid(50_000);
    for width in [1201.0, 1260.0, 1100.0] {
        let viewport = LayoutViewport::new(width, 800.0);
        let small_cost = Cost::from(small.resize(viewport));
        let huge_cost = Cost::from(huge.resize(viewport));
        assert_eq!(small_cost, huge_cost, "{width}");
        assert_eq!(
            (small_cost.local_fallbacks, small_cost.full_fallbacks),
            (0, 0)
        );
        assert!(small_cost.constraint_seeds > 0, "{width}: {small_cost:?}");
        for id in small.subtree(small.parts.dock) {
            assert!(
                !admitted(&small.context, id),
                "{width}: dock node {id:?} laid out"
            );
        }
    }
    let mut cold = Workspace::fluid(50_000);
    cold.resize(LayoutViewport::new(1100.0, 800.0));
    assert_matches_cold(&mut huge.context, &mut cold.context, huge.document);
}

/// A Dock resizes its frames through their flex factors. A factor sizes a
/// frame along its parent's line only: in a row, the boxes under it that
/// read the frame's width measure again, and the ones that read only its
/// height do not.
#[test]
fn issue264_a_flex_factor_resize_reaches_only_the_line_axis() {
    let document = DocumentId::new(1).unwrap();
    let (mut b, root) = Builder::new(document, 1);
    let row = b.element(
        root,
        LayoutStyle {
            direction: Some(FlexDirection::Row),
            align_items: AlignSpec::Stretch,
            ..fixed(900.0, 600.0)
        },
    );
    let frame = |grow: f32| LayoutStyle {
        flex_grow: Some(grow),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    };
    let left = b.element(row, frame(0.5));
    let right = b.element(row, frame(0.5));
    let mut wide = Vec::new();
    let mut tall = Vec::new();
    for pane in [left, right] {
        let reader = b.element(pane, fill_column());
        b.label(reader, PARAGRAPH);
        wide.push(reader);
        tall.push(b.element(
            pane,
            LayoutStyle {
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Percent(25.0)),
                ..LayoutStyle::default()
            },
        ));
    }
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    let mut shaper = bundled_face_shaper();
    product_frame(&mut context, document, viewport(), &mut shaper);
    for step in 1..=20u32 {
        let ratio = 0.5 + step as f32 / 100.0;
        let mut queue = MutationQueue::new();
        queue.set_style(left, styled(frame(ratio)));
        queue.set_style(right, styled(frame(1.0 - ratio)));
        context.commit_mutations(queue).unwrap();
        let counters = product_frame(&mut context, document, viewport(), &mut shaper);
        assert_eq!(counters.layout_full_document_fallbacks, 0);
        assert_eq!(counters.resize_text_reshapes, 0);
        for id in &tall {
            assert!(
                !measured(&context, *id),
                "{ratio}: height reader {id:?} measured"
            );
        }
        assert!(
            wide.iter().any(|id| measured(&context, *id)),
            "{ratio}: no width reader measured"
        );
    }
}
