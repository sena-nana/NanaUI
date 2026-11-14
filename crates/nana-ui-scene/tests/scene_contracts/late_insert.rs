//! A subtree created in one commit and inserted in a later one: with a
//! frame between, with none, and created detached. After the insert the
//! page holds what a page built with the subtree in place from the start
//! holds -- document order, every box and result, the hit index at every
//! point of a grid, the accessibility tree (projected whole and as the
//! deltas handed it out) and every scene primitive.
//!
//! A node created with no parent is a root of its document until it is
//! inserted (an application's page root is made that way); one detached is
//! not. A page that holds a subtree out of the tree for later detaches it.

use std::collections::BTreeMap;
use std::sync::Arc;

use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, SemanticColorRole};
use nana_ui_runtime::{
    AccessibilityNode, DocumentId, LayoutViewport, MeasureTextShaper, MutationQueue, NodeKind,
    NodeStyle, StableNodeId, TextContent,
};
use nana_ui_scene::{RuntimeDocument, UiScene};

const DOCUMENT: u64 = 1;
const PAGE: u64 = 2;
/// The page's rows; the subtree goes before the second.
const ROWS: [u64; 3] = [10, 11, 12];
/// The subtree inserted late: a row, a cell in it, a label in the cell.
const LATE: u64 = 50;
const CELL: u64 = 51;
const LABEL: u64 = 52;

fn document_id() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn id(raw: u64) -> StableNodeId {
    StableNodeId::new(raw).unwrap()
}

fn style(height: f32) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(LayoutStyle {
            width: Some(LengthSpec::Px(200.0)),
            height: Some(LengthSpec::Px(height)),
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            ..LayoutStyle::default()
        }),
        background: Some(SemanticColorRole::Accent),
        ..NodeStyle::default()
    }
}

fn element(queue: &mut MutationQueue, node: u64, parent: Option<u64>, height: f32) {
    queue.create(
        id(node),
        document_id(),
        NodeKind::Element { tag: "div".into() },
    );
    if let Some(parent) = parent {
        queue.insert(id(parent), id(node), None);
    }
    queue.set_style(id(node), style(height));
}

fn page(queue: &mut MutationQueue) {
    queue.create(id(DOCUMENT), document_id(), NodeKind::Document);
    element(queue, PAGE, Some(DOCUMENT), 400.0);
    for row in ROWS {
        element(queue, row, Some(PAGE), 20.0);
    }
}

/// The late subtree, with no parent.
fn late(queue: &mut MutationQueue) {
    element(queue, LATE, None, 30.0);
    element(queue, CELL, Some(LATE), 10.0);
    queue.create(id(LABEL), document_id(), NodeKind::Text);
    queue.insert(id(CELL), id(LABEL), None);
    queue.set_text(
        id(LABEL),
        TextContent {
            value: "Late".into(),
        },
    );
}

fn insert(queue: &mut MutationQueue) {
    queue.insert(id(PAGE), id(LATE), Some(id(ROWS[1])));
}

struct Page {
    runtime: RuntimeDocument,
    /// What the deltas handed out, applied one over the other.
    accessibility: BTreeMap<StableNodeId, AccessibilityNode>,
}

impl Page {
    fn new() -> Self {
        Self {
            runtime: RuntimeDocument::new(document_id()),
            accessibility: BTreeMap::new(),
        }
    }

    fn commit(&mut self, write: impl FnOnce(&mut MutationQueue)) {
        let mut queue = MutationQueue::new();
        write(&mut queue);
        self.runtime.context_mut().commit_mutations(queue).unwrap();
    }

    fn flush(&mut self) {
        let update = self
            .runtime
            .flush(LayoutViewport::new(400.0, 600.0), &mut MeasureTextShaper)
            .unwrap();
        for removed in &update.accessibility.removed {
            self.accessibility.remove(removed);
        }
        for node in update.accessibility.updated {
            self.accessibility.insert(node.id, node);
        }
    }

    fn order(&self) -> Vec<StableNodeId> {
        self.runtime.context().world().document_order(document_id())
    }
}

/// The page built with the late subtree in place, in one commit.
fn cold() -> Page {
    let mut cold = Page::new();
    cold.commit(|queue| {
        page(queue);
        late(queue);
        insert(queue);
    });
    cold.flush();
    cold
}

#[track_caller]
fn same(warm: &mut Page, at: &str) {
    let cold = cold();
    let (ours, theirs) = (warm.order(), cold.order());
    assert_eq!(ours, theirs, "{at}: document order");
    assert!(
        ours.contains(&id(LABEL)),
        "{at}: the late label is in order"
    );
    let (world, cold_world) = (
        warm.runtime.context().world(),
        cold.runtime.context().world(),
    );
    for &node in &theirs {
        assert_eq!(
            world.layout_box(node),
            cold_world.layout_box(node),
            "{at}: box of {node:?}"
        );
        match (
            world.local_layout_result(node),
            cold_world.local_layout_result(node),
        ) {
            (Some(ours), Some(theirs)) => assert!(
                ours.geometry_eq(theirs),
                "{at}: result of {node:?}: {ours:?} != {theirs:?}"
            ),
            (ours, theirs) => assert_eq!(ours, theirs, "{at}: result of {node:?}"),
        }
    }
    for row in 0..60 {
        for column in 0..40 {
            let (x, y) = (column as f32 * 10.3 + 0.4, row as f32 * 7.1 + 0.3);
            assert_eq!(
                world.hit_test_candidates(document_id(), x, y),
                cold_world.hit_test_candidates(document_id(), x, y),
                "{at}: hits at ({x}, {y})"
            );
        }
    }
    let full: BTreeMap<_, _> = cold_world
        .project_accessibility(document_id())
        .into_iter()
        .map(|node| (node.id, node))
        .collect();
    let projected: BTreeMap<_, _> = world
        .project_accessibility(document_id())
        .into_iter()
        .map(|node| (node.id, node))
        .collect();
    assert_eq!(projected, full, "{at}: the accessibility tree");
    assert_eq!(
        warm.accessibility, full,
        "{at}: the accessibility deltas add up to another tree"
    );
    let primitives = |scene: &UiScene| {
        scene
            .primitives()
            .map(|primitive| format!("{primitive:?}"))
            .collect::<Vec<_>>()
    };
    let mut fresh = UiScene::new();
    fresh.apply_delta(cold_world.extract_document(document_id()), []);
    assert_eq!(
        primitives(warm.runtime.scene()),
        primitives(&fresh),
        "{at}: the scene draws other primitives than a cold one"
    );
}

/// Created in one frame, inserted in the next: until then it is a root of
/// the document, laid out and painted as one.
#[test]
fn a_subtree_created_in_one_frame_is_inserted_in_the_next() {
    let mut warm = Page::new();
    warm.commit(page);
    warm.flush();
    warm.commit(late);
    warm.flush();
    assert_eq!(warm.runtime.context().world().parent_id(id(LATE)), None);
    assert!(
        warm.order().ends_with(&[id(LATE), id(CELL), id(LABEL)]),
        "a node created with no parent is a root of its document"
    );
    warm.commit(insert);
    warm.flush();
    same(&mut warm, "inserted the frame after");
    // Out, and back in.
    warm.commit(|queue| queue.detach(id(LATE)));
    warm.flush();
    assert!(!warm.order().contains(&id(LATE)), "detached");
    warm.commit(insert);
    warm.flush();
    same(&mut warm, "inserted again");
}

/// Created and inserted in two commits with no frame between.
#[test]
fn a_subtree_created_and_inserted_in_two_commits_of_one_frame() {
    let mut warm = Page::new();
    warm.commit(page);
    warm.flush();
    warm.commit(late);
    warm.commit(insert);
    warm.flush();
    same(&mut warm, "two commits");
}

/// Created with the page in its first commit, inserted the frame after.
#[test]
fn a_subtree_created_with_the_page_is_inserted_the_frame_after() {
    let mut warm = Page::new();
    warm.commit(|queue| {
        page(queue);
        late(queue);
    });
    warm.flush();
    warm.commit(insert);
    warm.flush();
    same(&mut warm, "created with the page");
}

/// Created detached: out of the document, then inserted the frame after.
#[test]
fn a_subtree_created_detached_is_inserted_the_frame_after() {
    let mut warm = Page::new();
    warm.commit(page);
    warm.flush();
    warm.commit(|queue| {
        late(queue);
        queue.detach(id(LATE));
    });
    warm.flush();
    assert!(
        !warm.order().contains(&id(LATE)),
        "a detached subtree is not in the document"
    );
    warm.commit(insert);
    warm.flush();
    same(&mut warm, "created detached");
}
