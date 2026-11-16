//! Which boxes hold an in-flow line box (`UiWorld::holds_line_box_in`) is
//! kept across layout passes, and a pass drops an answer only up the
//! ancestors of a seed that may change it (`may_move_line_boxes`).
//!
//! A page of rows in a scroller, one a flex row aligned by first baselines
//! whose items hold their line boxes in different ways (a label two blocks
//! down, a bare box, an inline-block atom). Each edit below changes one input
//! of the answer or none: a size, text appearing and disappearing, `display`
//! (inline-level, `none`, block again), `position` and `float`, a row
//! inserted and removed, `align-items: baseline` on and off. Every step is
//! compared with the same inputs laid out cold (`assert_matches_cold`), and
//! every pass checks each retained answer against a fresh walk, those it
//! reads and those it keeps (`checked_line_boxes`,
//! `verify::retained_line_boxes_are_fresh`). A size-only edit settles no box
//! again.
#![cfg(test)]

use nana_ui_core::{AlignSpec, DisplaySpec, FlexDirection, LayoutStyle, LengthSpec};
use nana_ui_core::{OverflowSpec, PositionSpec};

use super::reflow_oracle::{Builder, assert_matches_cold, bundled_face_shaper, product_frame};
use crate::{
    AppContext, DocumentId, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeStyle,
    StableNodeId, TextContent,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

/// The inputs a page is built from: what the edits change.
#[derive(Debug, Clone, Copy)]
struct Spec {
    /// The baseline row's bare box.
    leaf_min_height: f32,
    leaf_display: Option<DisplaySpec>,
    /// A bare row's leaf.
    row_leaf_min_height: f32,
    /// The text two blocks down the baseline row's first item.
    label: &'static str,
    /// The first item omits its box.
    item_hidden: bool,
    /// The third item's inline-flex atom.
    atom_position: PositionSpec,
    /// The extra row stands before the third row; otherwise it is detached.
    extra_row: bool,
    /// The second row aligns its items by their first baselines.
    baseline: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            leaf_min_height: 16.0,
            leaf_display: None,
            row_leaf_min_height: 16.0,
            label: "Label",
            item_hidden: false,
            atom_position: PositionSpec::Static,
            extra_row: false,
            baseline: true,
        }
    }
}

/// What the edits address.
#[derive(Debug, Clone, Copy)]
struct Ids {
    list: StableNodeId,
    baseline_row: StableNodeId,
    item: StableNodeId,
    label: StableNodeId,
    leaf: StableNodeId,
    atom: StableNodeId,
    extra: StableNodeId,
    before: StableNodeId,
    row_leaf: StableNodeId,
}

fn style(layout: LayoutStyle) -> NodeStyle {
    super::reflow_oracle::styled(layout)
}

fn row_style(baseline: bool) -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::Flex),
        direction: Some(FlexDirection::Row),
        align_items: if baseline {
            AlignSpec::Baseline
        } else {
            AlignSpec::default()
        },
        ..LayoutStyle::default()
    }
}

fn block(hidden: bool) -> LayoutStyle {
    LayoutStyle {
        hidden,
        ..LayoutStyle::default()
    }
}

fn leaf_style(min_height: f32, display: Option<DisplaySpec>) -> LayoutStyle {
    LayoutStyle {
        display,
        width: Some(LengthSpec::Px(24.0)),
        min_height: Some(LengthSpec::Px(min_height)),
        ..LayoutStyle::default()
    }
}

fn atom_style(position: PositionSpec) -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::InlineFlex),
        width: Some(LengthSpec::Px(20.0)),
        height: Some(LengthSpec::Px(10.0)),
        position,
        ..LayoutStyle::default()
    }
}

/// A scroller holding a list of six rows (and one more, detached unless
/// `spec.extra_row`), laid out once. Ids are handed out in the same order
/// whatever `spec` says, so a warm page and a cold one agree on them.
fn page(spec: Spec) -> (AppContext, NanaTextEngineShaper, Ids) {
    let (mut b, page) = Builder::page(document(), 1, 400.0);
    let scroller = b.element(
        page,
        LayoutStyle {
            direction: Some(FlexDirection::Column),
            width: Some(LengthSpec::Px(400.0)),
            height: Some(LengthSpec::Px(120.0)),
            overflow_y: OverflowSpec::Auto,
            ..LayoutStyle::default()
        },
    );
    let list = b.element(
        scroller,
        LayoutStyle {
            direction: Some(FlexDirection::Column),
            width: Some(LengthSpec::Px(400.0)),
            ..LayoutStyle::default()
        },
    );
    let first = b.element(list, row_style(false));
    let cell = b.element(first, block(false));
    b.label(cell, "Row");
    let baseline_row = b.element(list, row_style(spec.baseline));
    let item = b.element(baseline_row, block(spec.item_hidden));
    let inner = b.element(item, block(false));
    let label = b.label(inner, spec.label);
    let bare = b.element(baseline_row, block(false));
    let leaf = b.element(bare, leaf_style(spec.leaf_min_height, spec.leaf_display));
    let holder = b.element(baseline_row, block(false));
    let atom = b.element(holder, atom_style(spec.atom_position));
    let mut rows = Vec::new();
    let mut leaves = Vec::new();
    for index in 0..4 {
        let row = b.element(list, row_style(false));
        let cell = b.element(row, block(false));
        let height = if index == 2 {
            spec.row_leaf_min_height
        } else {
            16.0
        };
        rows.push(row);
        leaves.push(b.element(cell, leaf_style(height, None)));
    }
    let before = rows[0];
    let extra = b.detached(row_style(false));
    let extra_cell = b.element(extra, block(false));
    b.label(extra_cell, "Extra");
    // Out of the tree unless `spec.extra_row`: detached, as a warm page's
    // removed row is. Left with no parent it would be a root of the
    // document.
    if spec.extra_row {
        b.queue.insert(list, extra, Some(before));
    } else {
        b.queue.detach(extra);
    }
    let row_leaf = leaves[2];
    let queue = std::mem::take(&mut b.queue);
    let mut context = AppContext::new();
    context.commit_mutations(queue).unwrap();
    let mut shaper = bundled_face_shaper();
    product_frame(&mut context, document(), viewport(), &mut shaper);
    (
        context,
        shaper,
        Ids {
            list,
            baseline_row,
            item,
            label,
            leaf,
            atom,
            extra,
            before,
            row_leaf,
        },
    )
}

/// Commit `edit`, run a product frame, and compare with `spec` laid out cold.
/// Returns the boxes the pass's line-box questions settled by walking.
fn step(
    warm: &mut AppContext,
    shaper: &mut NanaTextEngineShaper,
    spec: Spec,
    edit: impl FnOnce(&mut MutationQueue),
) -> usize {
    let mut queue = MutationQueue::new();
    edit(&mut queue);
    warm.commit_mutations(queue).unwrap();
    product_frame(warm, document(), viewport(), shaper);
    let settled = warm.layout_execution_stats(document()).line_boxes_settled;
    let (mut cold, _, _) = page(spec);
    assert_matches_cold(warm, &mut cold, document());
    settled
}

/// A size-only edit keeps every answer: none is settled again, and the list
/// above it keeps its own, as does every row.
#[test]
fn a_size_edit_keeps_every_line_box_answer() {
    let mut spec = Spec::default();
    let (mut warm, mut shaper, ids) = page(spec);
    for height in [24.0, 16.0, 31.0] {
        spec.row_leaf_min_height = height;
        let settled = step(&mut warm, &mut shaper, spec, |queue| {
            queue.set_style(ids.row_leaf, style(leaf_style(height, None)));
        });
        assert_eq!(settled, 0, "a row's min-height settles no line box again");
        spec.leaf_min_height = height;
        let settled = step(&mut warm, &mut shaper, spec, |queue| {
            queue.set_style(ids.leaf, style(leaf_style(height, None)));
        });
        assert_eq!(
            settled, 0,
            "a baseline item's min-height settles no line box again"
        );
    }
}

/// Text that empties and fills again takes the line box away and back: the
/// first item then holds none, and the row aligns by the others.
#[test]
fn text_that_empties_and_fills_moves_the_line_box() {
    let mut spec = Spec::default();
    let (mut warm, mut shaper, ids) = page(spec);
    for label in ["", "Back", "", "Label"] {
        spec.label = label;
        let settled = step(&mut warm, &mut shaper, spec, |queue| {
            queue.set_text(ids.label, TextContent::new(label));
        });
        assert!(settled > 0, "text {label:?} settles its ancestors again");
    }
}

/// `display`: the bare box turned inline-level gives its block a line box,
/// then block-level takes it away; the first item omitted and shown again.
#[test]
fn display_changes_move_the_line_box() {
    let mut spec = Spec::default();
    let (mut warm, mut shaper, ids) = page(spec);
    for display in [
        Some(DisplaySpec::InlineFlex),
        Some(DisplaySpec::Flex),
        Some(DisplaySpec::Inline),
        Some(DisplaySpec::None),
        None,
    ] {
        spec.leaf_display = display;
        step(&mut warm, &mut shaper, spec, |queue| {
            queue.set_style(ids.leaf, style(leaf_style(spec.leaf_min_height, display)));
        });
    }
    for hidden in [true, false] {
        spec.item_hidden = hidden;
        step(&mut warm, &mut shaper, spec, |queue| {
            queue.set_style(ids.item, style(block(hidden)));
        });
    }
}

/// The inline-flex atom leaves the flow (`absolute`) and comes back: its
/// block holds no line box while it is out.
#[test]
fn an_atom_leaving_the_flow_moves_the_line_box() {
    let mut spec = Spec::default();
    let (mut warm, mut shaper, ids) = page(spec);
    for position in [PositionSpec::Absolute, PositionSpec::Static] {
        spec.atom_position = position;
        step(&mut warm, &mut shaper, spec, |queue| {
            queue.set_style(ids.atom, style(atom_style(position)));
        });
    }
}

/// A row holding a label inserted into the list and removed again.
#[test]
fn rows_inserted_and_removed_move_the_line_box() {
    let mut spec = Spec::default();
    let (mut warm, mut shaper, ids) = page(spec);
    for _ in 0..2 {
        spec.extra_row = true;
        step(&mut warm, &mut shaper, spec, |queue| {
            queue.insert(ids.list, ids.extra, Some(ids.before));
        });
        spec.extra_row = false;
        step(&mut warm, &mut shaper, spec, |queue| {
            queue.detach(ids.extra);
        });
    }
    // And a row that holds a line box removed for good: the cold page keeps
    // it detached.
    spec.extra_row = true;
    step(&mut warm, &mut shaper, spec, |queue| {
        queue.insert(ids.list, ids.extra, Some(ids.before));
    });
    spec.extra_row = false;
    step(&mut warm, &mut shaper, spec, |queue| {
        queue.despawn_subtree(ids.extra);
    });
}

/// `align-items: baseline` off and on: the row stops and starts asking its
/// items' line boxes, whose answers a size edit in between keeps.
#[test]
fn baseline_alignment_reads_kept_answers() {
    let mut spec = Spec::default();
    let (mut warm, mut shaper, ids) = page(spec);
    for baseline in [false, true, false, true] {
        spec.baseline = baseline;
        step(&mut warm, &mut shaper, spec, |queue| {
            queue.set_style(ids.baseline_row, style(row_style(baseline)));
        });
        spec.leaf_min_height += 3.0;
        step(&mut warm, &mut shaper, spec, |queue| {
            queue.set_style(
                ids.leaf,
                style(leaf_style(spec.leaf_min_height, spec.leaf_display)),
            );
        });
        spec.label = if spec.label.is_empty() { "Label" } else { "" };
        let label = spec.label;
        step(&mut warm, &mut shaper, spec, |queue| {
            queue.set_text(ids.label, TextContent::new(label));
        });
    }
}

/// Paragraphs that only move keep their line boxes, and everything else
/// under them.
mod moved_paragraphs;
