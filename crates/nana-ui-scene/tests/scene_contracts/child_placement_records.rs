//! A box whose own geometry stays while its children move inside it -- a
//! fixed-height column whose first row grows, a fixed-width row whose first
//! item widens -- publishes a result that changed only in where its
//! children sit. That projects nothing new of its own, but the record the
//! scene holds carries the result, so the box is extracted again. After
//! every frame each record the scene holds equals a fresh extraction; the
//! next frame an ancestor resizes and moves what holds the box, and with
//! `nana-ui-runtime/layout-verify` the runtime's own check of the records
//! kept under a move (`check_records_kept_under_moves`) runs as well.

use std::sync::Arc;

use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec};
use nana_ui_runtime::{
    DocumentId, LayoutViewport, MeasureTextShaper, MutationQueue, NodeKind, NodeStyle, StableNodeId,
};
use nana_ui_scene::RuntimeDocument;

/// The page's root, a column as tall as what it holds.
const ROOT: u64 = 1;
/// A row before the wrapper: growing it resizes the root and moves the
/// wrapper.
const LEAD: u64 = 2;
/// A column as tall as what it holds, around the fixed box.
const WRAPPER: u64 = 3;
/// The box of fixed size whose children move.
const FIXED: u64 = 4;
/// Its first child, which grows, and the one after, which moves.
const FIRST: u64 = 5;
const SECOND: u64 = 6;
/// The second child's own child, kept under its move.
const INNER: u64 = 7;

fn document_id() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn id(raw: u64) -> StableNodeId {
    StableNodeId::new(raw).unwrap()
}

fn styled(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

fn boxed(direction: FlexDirection, width: Option<f32>, height: Option<f32>) -> NodeStyle {
    styled(LayoutStyle {
        width: width.map(LengthSpec::Px),
        height: height.map(LengthSpec::Px),
        direction: Some(direction),
        flex_shrink: Some(0.0),
        padding_top: Some(LengthSpec::Px(0.7)),
        ..LayoutStyle::default()
    })
}

fn element(queue: &mut MutationQueue, node: u64, parent: Option<u64>, style: NodeStyle) {
    queue.create(
        id(node),
        document_id(),
        NodeKind::Element { tag: "div".into() },
    );
    if let Some(parent) = parent {
        queue.insert(id(parent), id(node), None);
    }
    queue.set_style(id(node), style);
}

/// The fixed box lays its children along `direction`, each `size` long on
/// that axis.
fn child(direction: FlexDirection, size: f32) -> NodeStyle {
    match direction {
        FlexDirection::Column => boxed(FlexDirection::Row, Some(200.0), Some(size)),
        FlexDirection::Row => boxed(FlexDirection::Row, Some(size), Some(20.0)),
    }
}

fn build(direction: FlexDirection) -> RuntimeDocument {
    let mut runtime = RuntimeDocument::new(document_id());
    let mut queue = MutationQueue::new();
    element(
        &mut queue,
        ROOT,
        None,
        boxed(FlexDirection::Column, Some(360.0), None),
    );
    element(
        &mut queue,
        LEAD,
        Some(ROOT),
        boxed(FlexDirection::Row, Some(300.0), Some(20.0)),
    );
    element(
        &mut queue,
        WRAPPER,
        Some(ROOT),
        boxed(FlexDirection::Column, Some(320.0), None),
    );
    element(
        &mut queue,
        FIXED,
        Some(WRAPPER),
        boxed(direction, Some(300.0), Some(60.0)),
    );
    element(&mut queue, FIRST, Some(FIXED), child(direction, 20.0));
    element(&mut queue, SECOND, Some(FIXED), child(direction, 20.0));
    element(
        &mut queue,
        INNER,
        Some(SECOND),
        boxed(FlexDirection::Row, Some(10.0), Some(5.0)),
    );
    runtime.context_mut().commit_mutations(queue).unwrap();
    flush(&mut runtime);
    runtime
}

fn flush(runtime: &mut RuntimeDocument) {
    runtime
        .flush(LayoutViewport::new(400.0, 600.0), &mut MeasureTextShaper)
        .unwrap();
}

fn edit(runtime: &mut RuntimeDocument, node: u64, style: NodeStyle) {
    let mut queue = MutationQueue::new();
    queue.set_style(id(node), style);
    runtime.context_mut().commit_mutations(queue).unwrap();
    flush(runtime);
}

/// Every record the scene holds is the one a fresh extraction gives.
#[track_caller]
fn records_fresh(runtime: &RuntimeDocument, at: &str) {
    let fresh = runtime.context().world().extract_document(document_id());
    assert!(!fresh.is_empty());
    for node in fresh {
        let held = runtime
            .scene()
            .extracted_node(node.id)
            .unwrap_or_else(|| panic!("{at}: the scene holds no record of {:?}", node.id));
        assert_eq!(
            held.layout_result, node.layout_result,
            "{at}: the scene holds {:?} with another result than a fresh extraction",
            node.id
        );
    }
}

fn grows_and_then_moves(direction: FlexDirection) {
    let mut runtime = build(direction);
    records_fresh(&runtime, "built");
    let mut size = 20.0;
    let mut lead = 20.0;
    for round in 0..3 {
        size += 1.37;
        edit(&mut runtime, FIRST, child(direction, size));
        let fixed = runtime.context().world().layout_box(id(FIXED)).unwrap();
        assert_eq!(
            (fixed.width, fixed.height),
            (300.0, 60.0),
            "round {round}: the fixed box keeps its size"
        );
        records_fresh(&runtime, &format!("round {round}: the first child grew"));
        // The root resizes and the wrapper moves; the fixed box, kept
        // under the move, keeps its record.
        lead += 1.37;
        edit(
            &mut runtime,
            LEAD,
            boxed(FlexDirection::Row, Some(300.0), Some(lead)),
        );
        records_fresh(&runtime, &format!("round {round}: an ancestor resized"));
    }
}

#[test]
fn a_fixed_column_whose_first_row_grows_is_extracted_again() {
    grows_and_then_moves(FlexDirection::Column);
}

#[test]
fn a_fixed_row_whose_first_item_widens_is_extracted_again() {
    grows_and_then_moves(FlexDirection::Row);
}
