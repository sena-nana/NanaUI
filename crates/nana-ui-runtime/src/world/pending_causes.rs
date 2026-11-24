//! A layout cause is queued per node and drained per document. One queued
//! for a node that then leaves the world belongs to no document: it must go
//! with the node, or it stays in the table for the life of the session and
//! every frame's drain looks at it again. Seventy thousand removed nodes
//! made each frame of a thousand-box page 3 ms slower.
#![cfg(test)]

use nana_ui_core::LayoutInvalidation;

use super::reflow_oracle::{bundled_face_shaper, product_frame};
use super::{DocumentId, StableNodeId};
use crate::{AppContext, LayoutFrontierSeed, LayoutViewport, Stack, Text};

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn frame(cx: &mut AppContext) {
    product_frame(
        cx,
        document(),
        LayoutViewport::new(700.0, 600.0),
        &mut bundled_face_shaper(),
    );
}

fn pending(cx: &AppContext) -> usize {
    cx.world().invalidation.pending_layout_invalidations.len()
}

/// A column of `rows` labels, laid out.
fn page(rows: usize) -> (AppContext, crate::Entity<Stack>, Vec<crate::Entity<Text>>) {
    let mut cx = AppContext::new();
    let root = cx.create_component(document(), Stack::column(0.0)).unwrap();
    let rows = (0..rows)
        .map(|index| {
            let row = cx
                .create_detached_component(document(), Text::new(format!("row {index}")))
                .unwrap();
            cx.append_child(root, row).unwrap();
            row
        })
        .collect();
    frame(&mut cx);
    assert_eq!(pending(&cx), 0, "a frame drains its document's causes");
    (cx, root, rows)
}

#[test]
fn a_removed_node_takes_its_pending_cause_with_it() {
    let (mut cx, root, mut rows) = page(50);

    // Rows that were laid out: the removal queues a cause for each.
    let removed: Vec<StableNodeId> = rows
        .drain(..25)
        .map(|row| {
            cx.remove_view(row).unwrap();
            row.stable_id()
        })
        .collect();
    for row in &removed {
        assert!(
            cx.world().pending_layout_invalidation(*row).is_empty(),
            "{row:?} is gone and still has a cause pending"
        );
    }
    assert!(pending(&cx) <= 1, "only their parent has one");
    frame(&mut cx);
    assert_eq!(pending(&cx), 0);

    // Rows appended and removed between two frames, and ones never attached.
    for index in 0..40 {
        let row = cx
            .create_detached_component(document(), Text::new(format!("gone {index}")))
            .unwrap();
        if index % 2 == 0 {
            cx.append_child(root, row).unwrap();
        }
        cx.remove_view(row).unwrap();
    }
    assert!(pending(&cx) <= 1, "only their parent has one");
    frame(&mut cx);
    assert_eq!(pending(&cx), 0);
    assert_eq!(cx.world().len(), 1 + 25, "the column and the rows left");
}

/// A cause that reaches the table for a node no longer in the world (a seed
/// handed back late, say) is dropped by the next drain of any document,
/// where one for a node of another document waits for its own.
#[test]
fn a_drain_drops_causes_of_nodes_that_left_the_world() {
    let (mut cx, _root, mut rows) = page(4);
    let other = DocumentId::new(2).unwrap();
    let elsewhere = cx.create_component(other, Stack::column(0.0)).unwrap();
    let gone = rows.pop().unwrap();
    cx.remove_view(gone).unwrap();
    let gone = gone.stable_id();
    frame(&mut cx);

    let world = cx.compat_world_mut();
    let cause = LayoutFrontierSeed::layout(gone).invalidation;
    world.queue_layout_invalidation(gone, cause);
    assert!(
        !world
            .pending_layout_invalidation(elsewhere.stable_id())
            .is_empty()
    );
    let work = world.take_system_work_for(document());
    assert!(
        work.layout_frontier_seeds
            .iter()
            .all(|seed| seed.node != gone)
    );
    assert_eq!(
        world.pending_layout_invalidation(gone),
        LayoutInvalidation::none()
    );
    assert!(
        !world
            .pending_layout_invalidation(elsewhere.stable_id())
            .is_empty(),
        "the other document's cause waits for its own drain"
    );
}
