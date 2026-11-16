//! A layout pass's writeback commits compact placements
//! ([`crate::mutation::LayoutPlacement`]) rather than a queue of
//! [`UiMutation::PlaceLayout`]. The two are one commit: these tests put the
//! same boxes through both and compare what each leaves.

use super::reflow_oracle::{bundled_face_shaper, column, fixed, node, product_frame, styled};
use super::*;
use crate::mutation::LayoutPlacement;
use crate::{AppContext, FrameworkError, LayoutViewport, MutationQueue, TextContent, UiMutation};
use nana_ui_core::{LengthSpec, OverflowSpec};

const PARENT: u64 = 2;
const ROWS: std::ops::Range<u64> = 10..22;
const LABELS: u64 = 100_000;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

/// A scrolling column of labelled rows, laid out once.
fn page() -> AppContext {
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(), NodeKind::Document);
    queue.create(
        node(PARENT),
        document(),
        NodeKind::Element { tag: "div".into() },
    );
    queue.insert(node(1), node(PARENT), None);
    let mut parent = column(Some(400.0));
    parent.height = Some(LengthSpec::Px(120.0));
    parent.overflow_y = OverflowSpec::Scroll;
    queue.set_style(node(PARENT), styled(parent));
    for row in ROWS {
        queue.create(
            node(row),
            document(),
            NodeKind::Element { tag: "div".into() },
        );
        queue.insert(node(PARENT), node(row), None);
        queue.set_style(
            node(row),
            styled(fixed(200.0, 12.0 + (row % 7) as f32 * 3.0)),
        );
        let label = node(LABELS + row);
        queue.create(label, document(), NodeKind::Text);
        queue.insert(node(row), label, None);
        queue.set_text(
            label,
            TextContent {
                value: format!("row {row}").into(),
            },
        );
    }
    let mut cx = AppContext::new();
    cx.commit_mutations(queue).unwrap();
    product_frame(
        &mut cx,
        document(),
        LayoutViewport::new(700.0, 600.0),
        &mut bundled_face_shaper(),
    );
    cx.compat_world_mut().take_system_work();
    cx
}

/// Boxes a pass could write back over [`page`]: every third row taller and
/// everything after it lower, the last row where it was, the column wider.
fn placements(world: &UiWorld) -> Vec<LayoutPlacement> {
    let mut placements = Vec::new();
    let mut down = 0.0;
    for row in ROWS {
        let id = node(row);
        let mut layout = world.local_layout_box(id).unwrap();
        let frame = world.layout_frame(id).unwrap();
        layout.y += down;
        if row % 3 == 0 {
            layout.height += 9.0;
            down += 9.0;
        }
        placements.push(LayoutPlacement { id, layout, frame });
    }
    let id = node(PARENT);
    let mut layout = world.local_layout_box(id).unwrap();
    layout.width += 20.0;
    placements.push(LayoutPlacement {
        id,
        layout,
        frame: world.layout_frame(id).unwrap(),
    });
    placements
}

fn queued(placements: &[LayoutPlacement]) -> MutationQueue {
    let mut queue = MutationQueue::new();
    for placement in placements {
        queue.push(UiMutation::PlaceLayout {
            id: placement.id,
            layout: placement.layout,
            frame: placement.frame,
        });
    }
    queue
}

/// Everything a commit leaves that the frame after it reads.
fn left(cx: &mut AppContext) -> impl PartialEq + std::fmt::Debug {
    let world = cx.compat_world_mut();
    world.flush_scroll_content();
    let nodes: Vec<_> = world
        .document_order(document())
        .into_iter()
        .map(|id| {
            (
                id,
                world.layout_box(id),
                world.local_layout_box(id),
                world.layout_frame(id),
                world.has_local_layout_result(id),
                world.scroll_content_bounds.borrow().stale(id),
                world.scroll_metrics(id),
            )
        })
        .collect();
    (
        world.generation(),
        world.take_structural_change_parents(),
        nodes,
        world.take_system_work(),
    )
}

/// The same boxes, queued as mutations and handed over as placements, are
/// the same commit: the same report, and the same world for the next frame.
/// Whether the pass publishes the written boxes itself or leaves that to the
/// commit makes two cases.
#[test]
fn placements_commit_as_the_queued_mutations_do() {
    for pass_publishes in [false, true] {
        let mut by_queue = page();
        let mut by_placements = page();
        let placements = placements(by_queue.world());
        assert_eq!(placements, self::placements(by_placements.world()));
        for cx in [&mut by_queue, &mut by_placements] {
            cx.compat_world_mut()
                .set_pass_publishes_written_boxes(pass_publishes);
        }
        let queued_report = by_queue.commit_mutations(queued(&placements)).unwrap();
        let placed_report = by_placements.commit_layout_placements(&placements).unwrap();
        for cx in [&mut by_queue, &mut by_placements] {
            cx.compat_world_mut()
                .set_pass_publishes_written_boxes(false);
        }
        assert_eq!(queued_report, placed_report, "publishes {pass_publishes}");
        assert_eq!(queued_report.mutations, placements.len());
        assert_eq!(
            left(&mut by_queue),
            left(&mut by_placements),
            "publishes {pass_publishes}"
        );
        // The boxes did move: the comparison above is not of two untouched
        // worlds.
        assert_eq!(
            by_placements.world().local_layout_box(node(PARENT)),
            Some(placements.last().unwrap().layout)
        );
    }
}

/// No placement at all is a commit that changes nothing, as an empty queue
/// is: the generation stays.
#[test]
fn no_placements_commit_as_an_empty_queue_does() {
    let mut by_queue = page();
    let mut by_placements = page();
    let queued_report = by_queue.commit_mutations(MutationQueue::new()).unwrap();
    let placed_report = by_placements.commit_layout_placements(&[]).unwrap();
    assert_eq!(queued_report, placed_report);
    assert_eq!(left(&mut by_queue), left(&mut by_placements));
}

/// A batch that names a node the world does not have, or a box that is no
/// box, is refused whole with the error the queued mutations get, and
/// leaves the world as it was.
#[test]
fn a_refused_batch_of_placements_changes_nothing() {
    for bad in ["missing node", "invalid box"] {
        let mut by_queue = page();
        let mut by_placements = page();
        let mut untouched = page();
        let mut placements = placements(by_queue.world());
        match bad {
            "missing node" => placements[5].id = node(9_999),
            _ => placements[5].layout.width = f32::NAN,
        }
        let queued = by_queue.commit_mutations(queued(&placements)).unwrap_err();
        let placed = by_placements
            .commit_layout_placements(&placements)
            .unwrap_err();
        match (&queued, &placed) {
            (FrameworkError::World(queued), FrameworkError::World(placed)) => {
                assert_eq!(queued, placed, "{bad}");
            }
            other => panic!("{bad}: {other:?}"),
        }
        let expected = left(&mut untouched);
        assert_eq!(left(&mut by_queue), expected, "{bad}");
        assert_eq!(left(&mut by_placements), expected, "{bad}");
    }
}
