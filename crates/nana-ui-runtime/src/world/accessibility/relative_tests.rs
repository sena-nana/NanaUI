//! Accessible bounds relative to the parent's: a box layout moved with its
//! parent keeps them, so a delta leaves it, and what it carries, alone.

use std::collections::BTreeMap;

use nana_ui_core::{
    FlexDirection, LayoutStyle, LengthSpec, OverflowSpec, PaintTransform, PositionSpec,
};

use super::super::reflow_oracle::{
    Builder, accessible_frame, bundled_face_shaper, column, fixed, product_frame, styled,
};
use crate::{
    AccessibilityNode, AppContext, DocumentId, LayoutViewport, MutationQueue, ScrollOffset,
    StableNodeId,
};

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(400.0, 600.0)
}

struct Page {
    context: AppContext,
    /// The rows, each a row holding a cell holding a leaf.
    rows: Vec<[StableNodeId; 3]>,
    /// A fixed box, and an absolute one, in the last row's cell, when asked
    /// for.
    hanging: Option<[StableNodeId; 2]>,
    /// The column the rows stand in.
    list: StableNodeId,
}

/// `rows` rows in a column, the column a scroller when `scroller`.
fn page(rows: usize, scroller: bool, hanging: bool) -> Page {
    let (mut b, page) = Builder::page(document(), 1, 400.0);
    let list = b.element(
        page,
        if scroller {
            LayoutStyle {
                overflow_x: OverflowSpec::Auto,
                overflow_y: OverflowSpec::Auto,
                direction: Some(FlexDirection::Column),
                flex_shrink: Some(0.0),
                ..fixed(300.0, 120.0)
            }
        } else {
            column(Some(300.0))
        },
    );
    let mut ids = Vec::new();
    for _ in 0..rows {
        let row = b.element(
            list,
            LayoutStyle {
                flex_shrink: Some(0.0),
                ..fixed(300.0, 20.0)
            },
        );
        let cell = b.element(
            row,
            LayoutStyle {
                position: PositionSpec::Relative,
                ..fixed(100.0, 10.0)
            },
        );
        let leaf = b.element(cell, fixed(50.0, 5.0));
        ids.push([row, cell, leaf]);
    }
    let cell = ids.last().unwrap()[1];
    let placed = |position, left: f32, top: f32| LayoutStyle {
        position,
        offset_left: Some(LengthSpec::Px(left)),
        offset_top: Some(LengthSpec::Px(top)),
        ..fixed(10.0, 10.0)
    };
    let hanging = hanging.then(|| {
        [
            b.element(cell, placed(PositionSpec::Fixed, 320.0, 10.0)),
            b.element(cell, placed(PositionSpec::Absolute, 60.0, 2.0)),
        ]
    });
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    product_frame(
        &mut context,
        document(),
        viewport(),
        &mut bundled_face_shaper(),
    );
    Page {
        context,
        rows: ids,
        hanging,
        list,
    }
}

fn snapshot(context: &AppContext) -> BTreeMap<StableNodeId, AccessibilityNode> {
    context
        .world()
        .project_accessibility(document())
        .into_iter()
        .map(|node| (node.id, node))
        .collect()
}

/// Commit `queue`, run the frame and apply its delta to `retained`, which
/// then equals a full projection. Returns the ids the delta updated.
fn edit(
    page: &mut Page,
    retained: &mut BTreeMap<StableNodeId, AccessibilityNode>,
    queue: MutationQueue,
    at: &str,
) -> Vec<StableNodeId> {
    page.context.commit_mutations(queue).unwrap();
    let work = accessible_frame(
        &mut page.context,
        document(),
        viewport(),
        &mut bundled_face_shaper(),
        false,
    );
    let delta = page.context.world().project_accessibility_delta(&work);
    for id in &delta.removed {
        retained.remove(id);
    }
    let updated = delta.updated.iter().map(|node| node.id).collect();
    for node in delta.updated {
        retained.insert(node.id, node);
    }
    let full = snapshot(&page.context);
    for (id, node) in &full {
        assert_eq!(
            retained.get(id),
            Some(node),
            "{at}: the delta left {id:?} unlike a full projection"
        );
    }
    // A closed menu's options are no one's children any more: a platform
    // tree drops them with the field's new child list, not by tombstone.
    retained.retain(|id, _| full.contains_key(id) || page.context.world().nodes.contains(*id));
    assert_eq!(
        retained.len(),
        full.len(),
        "{at}: the delta left nodes a full projection does not have"
    );
    updated
}

fn row_height(queue: &mut MutationQueue, row: StableNodeId, height: f32) {
    queue.set_style(
        row,
        styled(LayoutStyle {
            flex_shrink: Some(0.0),
            ..fixed(300.0, height)
        }),
    );
}

fn viewport_bounds(context: &AppContext, id: StableNodeId) -> crate::LayoutBox {
    context
        .world()
        .project_accessibility_in_viewport(document())
        .into_iter()
        .find(|node| node.id == id)
        .unwrap()
        .bounds
}

#[test]
fn a_grown_row_projects_the_rows_it_moves_but_not_what_they_hold() {
    let mut page = page(12, false, false);
    let mut retained = snapshot(&page.context);
    let leaf = page.rows[5][2];
    let before = viewport_bounds(&page.context, leaf);
    assert_eq!(
        retained[&leaf].bounds,
        crate::LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 5.0
        },
        "a leaf at its cell's corner stands at 0,0 from it"
    );
    for (height, at) in [(26.0, "grown"), (20.0, "back")] {
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        let updated = edit(&mut page, &mut retained, queue, at);
        for [row, cell, leaf] in &page.rows[1..] {
            assert!(updated.contains(row), "{at}: a moved row projects again");
            assert!(
                !updated.contains(cell) && !updated.contains(leaf),
                "{at}: what a moved row holds moved with it"
            );
        }
    }
    let mut queue = MutationQueue::new();
    row_height(&mut queue, page.rows[0][0], 26.0);
    edit(&mut page, &mut retained, queue, "grown again");
    let after = viewport_bounds(&page.context, leaf);
    assert_eq!((after.x, after.y), (before.x, before.y + 6.0));
}

/// A fixed box the viewport holds still while the cell it is in moves, so
/// its offset from the cell changes, and an absolute one that moves with the
/// cell and keeps it.
#[test]
fn boxes_hanging_from_a_moved_row_match_a_full_projection() {
    let mut page = page(12, false, true);
    let mut retained = snapshot(&page.context);
    let [fixed_box, absolute] = page.hanging.unwrap();
    let before = viewport_bounds(&page.context, fixed_box);
    for (height, at) in [(26.0, "grown"), (20.0, "back"), (33.0, "grown more")] {
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        let updated = edit(&mut page, &mut retained, queue, at);
        assert!(updated.contains(&fixed_box), "{at}: its offset changed");
        assert!(!updated.contains(&absolute), "{at}: its offset held");
    }
    assert_eq!(viewport_bounds(&page.context, fixed_box), before);
}

#[test]
fn moved_rows_under_a_scrolled_list_match_a_full_projection() {
    let mut page = page(12, true, true);
    let mut retained = snapshot(&page.context);
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(page.list, ScrollOffset { x: 0.0, y: 30.0 });
    edit(&mut page, &mut retained, queue, "scrolled");
    for (height, at) in [(26.0, "grown"), (20.0, "back"), (33.0, "grown more")] {
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[2][0], height);
        edit(&mut page, &mut retained, queue, at);
    }
    // A scroll and a move in one frame.
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(page.list, ScrollOffset { x: 0.0, y: 10.0 });
    row_height(&mut queue, page.rows[1][0], 24.0);
    edit(&mut page, &mut retained, queue, "scrolled and grown");
}

/// Boxes under moved rows that do not keep their offset from their parent
/// to the bit -- a transform, a sticky offset, a clip, a box measured from
/// the column above the row, a fixed and an absolute one -- each with a
/// plain box inside, on a page with a visual (so the projection takes the
/// path visuals do). The delta finds them from the pins alone and matches a
/// full projection field by field; the plain boxes around them stay put.
#[test]
fn pinned_boxes_under_moved_rows_match_a_full_projection() {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let list = b.element(
        root,
        LayoutStyle {
            position: PositionSpec::Relative,
            ..column(Some(300.0))
        },
    );
    let mut rows = Vec::new();
    for _ in 0..12 {
        let row = b.element(
            list,
            LayoutStyle {
                flex_shrink: Some(0.0),
                ..fixed(300.0, 20.0)
            },
        );
        let cell = b.element(
            row,
            LayoutStyle {
                position: PositionSpec::Relative,
                ..fixed(100.0, 10.0)
            },
        );
        let leaf = b.element(cell, fixed(50.0, 5.0));
        rows.push([row, cell, leaf]);
    }
    let placed = |position, left: f32, top: f32| LayoutStyle {
        position,
        offset_left: Some(LengthSpec::Px(left)),
        offset_top: Some(LengthSpec::Px(top)),
        ..fixed(10.0, 10.0)
    };
    let pins = [
        {
            let mut style = fixed(40.0, 8.0);
            style.transforms.transform = Some(PaintTransform {
                e: 3.25,
                f: 1.5,
                ..Default::default()
            });
            style
        },
        LayoutStyle {
            position: PositionSpec::Sticky,
            offset_top: Some(LengthSpec::Px(0.0)),
            ..fixed(40.0, 8.0)
        },
        LayoutStyle {
            overflow_x: OverflowSpec::Hidden,
            overflow_y: OverflowSpec::Hidden,
            ..fixed(40.0, 8.0)
        },
        // Measured from the column, above the row.
        placed(PositionSpec::Absolute, 210.0, 3.0),
        placed(PositionSpec::Fixed, 320.0, 10.0),
        // Measured from the row's own cell.
        placed(PositionSpec::Absolute, 60.0, 2.0),
    ];
    let mut pinned = Vec::new();
    for (index, style) in pins.into_iter().enumerate() {
        let [row, cell, _] = rows[5 + index];
        let parent = if index == 3 { row } else { cell };
        let pin = b.element(parent, style);
        let inside = b.element(pin, fixed(4.0, 3.0));
        pinned.push((pin, inside));
    }
    b.queue
        .set_widget_visual(rows[11][2], Some(crate::KeymapLayerVisual));
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    product_frame(
        &mut context,
        document(),
        viewport(),
        &mut bundled_face_shaper(),
    );
    let mut page = Page {
        context,
        rows,
        hanging: None,
        list,
    };
    let mut retained = snapshot(&page.context);
    for (height, at) in [(26.25, "grown"), (20.0, "back"), (33.5, "grown more")] {
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        let updated = edit(&mut page, &mut retained, queue, at);
        for [row, cell, leaf] in &page.rows[1..] {
            assert!(updated.contains(row), "{at}: a moved row projects again");
            assert!(
                !updated.contains(cell) && !updated.contains(leaf),
                "{at}: what a moved row holds moved with it"
            );
        }
        let (absolute, inside) = pinned[5];
        assert!(
            !updated.contains(&absolute) && !updated.contains(&inside),
            "{at}: a box measured from the moved cell kept its offset"
        );
        for &(pin, _) in &pinned[3..5] {
            assert!(updated.contains(&pin), "{at}: its frame did not move");
        }
    }
}

/// A box layout places from the page while its own style is in flow (an
/// open triggered menu's item) is noted as a pin when the frame is written,
/// and stops being one when it is placed from its parent again.
#[test]
fn a_box_placed_from_another_frame_is_noted_as_a_pin() {
    let mut page = page(2, false, false);
    let leaf = page.rows[1][2];
    let pinned = |context: &AppContext| context.world().nodes.space_pins().any(|id| id == leaf);
    assert!(!pinned(&page.context), "an in-flow box pins nothing");
    let local = page.context.world().local_layout_box(leaf).unwrap();
    for (frame, expected) in [
        (crate::LayoutFrame::Page, true),
        (crate::LayoutFrame::Parent, false),
    ] {
        let mut queue = MutationQueue::new();
        queue.place_layout(leaf, local, frame);
        page.context.commit_mutations(queue).unwrap();
        assert_eq!(pinned(&page.context), expected, "{frame:?}");
    }
}

/// Rows grown by an uneven step, again and again, move what they hold by
/// amounts whose page sums round differently each frame. A box that moved
/// with its parent is handed its offset from it, its local layout offset,
/// so the bounds it kept are a cold projection's to the bit -- with a
/// visual on the page, too, so the projection takes the path visuals do.
#[test]
fn boxes_moved_by_an_uneven_step_keep_a_cold_projection_to_the_bit() {
    for visual in [false, true] {
        let (mut b, root) = Builder::page(document(), 1, 400.0);
        let list = b.element(root, column(Some(300.0)));
        let mut rows = Vec::new();
        for _ in 0..12 {
            let row = b.element(
                list,
                LayoutStyle {
                    flex_shrink: Some(0.0),
                    ..fixed(300.0, 20.0)
                },
            );
            let cell = b.element(
                row,
                LayoutStyle {
                    padding_left: Some(LengthSpec::Px(1.3)),
                    padding_top: Some(LengthSpec::Px(2.5)),
                    ..fixed(100.0, 14.0)
                },
            );
            let leaf = b.element(cell, fixed(50.0, 5.0));
            rows.push([row, cell, leaf]);
        }
        if visual {
            b.queue
                .set_widget_visual(rows[11][2], Some(crate::KeymapLayerVisual));
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        product_frame(
            &mut context,
            document(),
            viewport(),
            &mut bundled_face_shaper(),
        );
        let mut page = Page {
            context,
            rows,
            hanging: None,
            list,
        };
        let mut retained = snapshot(&page.context);
        let mut height = 20.0;
        for step in 0..12 {
            height += 1.37;
            let at = format!("visual {visual}, step {step}");
            let mut queue = MutationQueue::new();
            row_height(&mut queue, page.rows[0][0], height);
            let updated = edit(&mut page, &mut retained, queue, &at);
            for [_, cell, leaf] in &page.rows[1..] {
                assert!(
                    !updated.contains(cell) && !updated.contains(leaf),
                    "{at}: what a moved row holds moved with it"
                );
                let local = page.context.world().local_layout_box(*leaf).unwrap();
                let bounds = retained[leaf].bounds;
                assert_eq!(
                    (bounds.x.to_bits(), bounds.y.to_bits()),
                    (local.x.to_bits(), local.y.to_bits()),
                    "{at}: a leaf is handed its offset in its cell"
                );
            }
        }
    }
}

/// `rows` rows standing in `list` under `root`, each a row holding a cell
/// (padded unevenly) holding a leaf; the rows are `relative` when `placed`.
fn uneven_rows(b: &mut Builder, list: StableNodeId, rows: usize) -> Vec<[StableNodeId; 3]> {
    (0..rows)
        .map(|_| {
            let row = b.element(
                list,
                LayoutStyle {
                    flex_shrink: Some(0.0),
                    position: PositionSpec::Relative,
                    ..fixed(300.0, 20.0)
                },
            );
            let cell = b.element(
                row,
                LayoutStyle {
                    padding_left: Some(LengthSpec::Px(1.3)),
                    padding_top: Some(LengthSpec::Px(2.5)),
                    ..fixed(100.0, 14.0)
                },
            );
            let leaf = b.element(cell, fixed(50.0, 5.0));
            [row, cell, leaf]
        })
        .collect()
}

fn laid_out(b: Builder, rows: Vec<[StableNodeId; 3]>, list: StableNodeId) -> Page {
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    product_frame(
        &mut context,
        document(),
        viewport(),
        &mut bundled_face_shaper(),
    );
    Page {
        context,
        rows,
        hanging: None,
        list,
    }
}

/// Grow the first row by an uneven step `steps` times, each a frame whose
/// delta must match a full projection; what the moved rows hold must not
/// project again. `each` sees the page after every step.
fn grow_unevenly(
    page: &mut Page,
    retained: &mut BTreeMap<StableNodeId, AccessibilityNode>,
    steps: usize,
    label: &str,
    mut each: impl FnMut(&Page, &BTreeMap<StableNodeId, AccessibilityNode>, &[StableNodeId], &str),
) {
    let mut height = 20.0;
    for step in 0..steps {
        height += 1.37;
        let at = format!("{label}, step {step}");
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        let updated = edit(page, retained, queue, &at);
        for [row, cell, leaf] in &page.rows[1..] {
            assert!(updated.contains(row), "{at}: a moved row projects again");
            assert!(
                !updated.contains(cell) && !updated.contains(leaf),
                "{at}: what a moved row holds moved with it"
            );
        }
        each(page, retained, &updated, &at);
    }
}

/// Rows in a list scrolled by an uneven amount, grown by an uneven step again
/// and again. Scrolling only translates, so a moved row is handed its offset
/// in the list's space moved by the list's scroll, which the list holds, and
/// what it holds its offset: the delta projects
/// the moved rows alone, from the pins, and what they kept is a cold
/// projection's to the bit -- with a visual on the page too, and after the
/// list scrolls again in a frame that also moves them.
#[test]
fn boxes_moved_in_a_scrolled_list_keep_a_cold_projection_to_the_bit() {
    for visual in [false, true] {
        let (mut b, root) = Builder::page(document(), 1, 400.0);
        let list = b.element(
            root,
            LayoutStyle {
                overflow_x: OverflowSpec::Auto,
                overflow_y: OverflowSpec::Auto,
                direction: Some(FlexDirection::Column),
                flex_shrink: Some(0.0),
                ..fixed(300.0, 120.0)
            },
        );
        let rows = uneven_rows(&mut b, list, 12);
        if visual {
            b.queue
                .set_widget_visual(rows[11][2], Some(crate::KeymapLayerVisual));
        }
        let mut page = laid_out(b, rows, list);
        let mut retained = snapshot(&page.context);
        let mut scroll = 30.37;
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(page.list, ScrollOffset { x: 0.0, y: scroll });
        edit(&mut page, &mut retained, queue, "scrolled");
        for round in 0..2 {
            let label = format!("visual {visual}, round {round}");
            grow_unevenly(
                &mut page,
                &mut retained,
                6,
                &label,
                |page, retained, _, at| {
                    let world = page.context.world();
                    for [row, _, leaf] in &page.rows[1..] {
                        let local = world.local_layout_box(*leaf).unwrap();
                        let bounds = retained[leaf].bounds;
                        assert_eq!(
                            (bounds.x.to_bits(), bounds.y.to_bits()),
                            (local.x.to_bits(), local.y.to_bits()),
                            "{at}: a leaf is handed its offset in its cell"
                        );
                        let local = world.local_layout_box(*row).unwrap();
                        let bounds = retained[row].bounds;
                        assert_eq!(
                            (bounds.y.to_bits(), bounds.height.to_bits()),
                            (local.y.to_bits(), local.height.to_bits()),
                            "{at}: a row is handed its offset in the list's scrolled space"
                        );
                    }
                    assert_eq!(
                        retained[&page.list].scroll,
                        [0.0, scroll],
                        "{at}: the list holds its scroll"
                    );
                },
            );
            // Scroll and move in one frame, then go on from there.
            scroll += 11.13;
            let mut queue = MutationQueue::new();
            queue.set_scroll_offset(page.list, ScrollOffset { x: 0.0, y: scroll });
            row_height(&mut queue, page.rows[0][0], 20.0);
            edit(
                &mut page,
                &mut retained,
                queue,
                &format!("{label}: scrolled and moved"),
            );
        }
    }
}

/// Rows under an ancestor that scales and translates them, moved by an uneven
/// step: the scale is handed to the platform as the ancestor's own
/// `diag(a, d)` transform, and every box under it is placed by its local
/// offset inside it, so the moved rows' contents keep their bounds to the
/// bit. Under a rotation the same holds; both match a full projection.
#[test]
fn boxes_moved_under_a_transformed_ancestor_match_a_cold_projection() {
    let (sin, cos) = 0.3f32.sin_cos();
    for (name, transform, offsets) in [
        (
            "scaled",
            PaintTransform {
                a: 1.5,
                d: 1.25,
                e: 3.3,
                f: 7.1,
                ..Default::default()
            },
            true,
        ),
        (
            "rotated",
            PaintTransform {
                a: cos,
                b: sin,
                c: -sin,
                d: cos,
                e: 2.0,
                f: 1.0,
            },
            false,
        ),
    ] {
        let (mut b, root) = Builder::page(document(), 1, 400.0);
        let mut wrapper_style = fixed(320.0, 500.0);
        wrapper_style.transforms.transform = Some(transform);
        let wrapper = b.element(root, wrapper_style);
        let list = b.element(wrapper, column(Some(300.0)));
        let rows = uneven_rows(&mut b, list, 12);
        let mut page = laid_out(b, rows, list);
        let mut retained = snapshot(&page.context);
        if offsets {
            grow_unevenly(
                &mut page,
                &mut retained,
                12,
                name,
                |page, retained, _, at| {
                    for [_, _, leaf] in &page.rows[1..] {
                        let local = page.context.world().local_layout_box(*leaf).unwrap();
                        let bounds = retained[leaf].bounds;
                        assert_eq!(
                            [bounds.x, bounds.y, bounds.width, bounds.height].map(f32::to_bits),
                            [local.x, local.y, local.width, local.height].map(f32::to_bits),
                            "{at}: a leaf is handed its offset in the scaled space"
                        );
                        assert_eq!(retained[leaf].transform, None, "{at}: an upright leaf");
                    }
                    let wrapper = page
                        .context
                        .world()
                        .nodes
                        .get(page.list)
                        .unwrap()
                        .hierarchy
                        .parent
                        .unwrap();
                    let handed = retained[&wrapper].transform.unwrap();
                    assert_eq!(
                        [handed.a, handed.b, handed.c, handed.d, handed.e, handed.f],
                        [transform.a, 0.0, 0.0, transform.d, 0.0, 0.0],
                        "{at}: the wrapper hands its scale as diag(a, d)"
                    );
                },
            );
        } else {
            let mut height = 20.0;
            for step in 0..12 {
                height += 1.37;
                let mut queue = MutationQueue::new();
                row_height(&mut queue, page.rows[0][0], height);
                edit(
                    &mut page,
                    &mut retained,
                    queue,
                    &format!("{name}, step {step}"),
                );
            }
        }
    }
}

/// Pins below pinned boxes that still hand their children offsets -- a
/// scaled box, a sticky one, and an absolute box measured from the moved row
/// above its parent -- are found from the pins too: each holds a box that
/// clips (whose subtree projects again) and a plain one (which keeps its
/// bounds).
#[test]
fn pins_under_boxes_that_hand_offsets_are_found_from_the_pins() {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let list = b.element(root, column(Some(300.0)));
    let rows = uneven_rows(&mut b, list, 12);
    let holders = [
        {
            let mut style = fixed(40.0, 8.0);
            style.transforms.transform = Some(PaintTransform {
                a: 2.0,
                d: 0.5,
                e: 1.25,
                ..Default::default()
            });
            style
        },
        LayoutStyle {
            position: PositionSpec::Sticky,
            offset_top: Some(LengthSpec::Px(0.0)),
            ..fixed(40.0, 8.0)
        },
        // The cell is static, so this one is measured from the row.
        LayoutStyle {
            position: PositionSpec::Absolute,
            offset_left: Some(LengthSpec::Px(60.0)),
            offset_top: Some(LengthSpec::Px(2.0)),
            ..fixed(30.0, 10.0)
        },
    ];
    let mut held = Vec::new();
    for (index, style) in holders.into_iter().enumerate() {
        let cell = rows[4 + index][1];
        let holder = b.element(cell, style);
        let clip = b.element(
            holder,
            LayoutStyle {
                overflow_x: OverflowSpec::Hidden,
                overflow_y: OverflowSpec::Hidden,
                ..fixed(20.0, 6.0)
            },
        );
        let clipped = b.element(clip, fixed(4.0, 3.0));
        let plain = b.element(holder, fixed(5.0, 2.0));
        held.push([holder, clip, clipped, plain]);
    }
    let mut page = laid_out(b, rows, list);
    assert!(matches!(
        page.context.world().layout_frame(held[2][0]),
        Some(crate::LayoutFrame::Block(_))
    ));
    let mut retained = snapshot(&page.context);
    grow_unevenly(&mut page, &mut retained, 6, "held", |_, _, updated, at| {
        for (index, [_, clip, clipped, plain]) in held.iter().enumerate() {
            assert!(
                updated.contains(clip) && updated.contains(clipped),
                "{at}: a moved clip projects what it holds again"
            );
            // A sticky box is marked whenever its scroller's geometry
            // changes, and projects its subtree again.
            if index != 1 {
                assert!(
                    !updated.contains(plain),
                    "{at}: a plain box kept its bounds"
                );
            }
        }
    });
}

/// A closed select projects its field alone, placed as any box is, so it
/// moves with its row and keeps its node; an open one places its list box
/// and options from its geometry, which is on the page, and projects again
/// under every move -- in a scrolled list, by an uneven step.
#[test]
fn a_closed_select_moves_with_its_row_and_an_open_one_projects_again() {
    let mut page = page(12, true, false);
    let mut select = crate::Select::new(Some("a"));
    select.options = vec![
        crate::SelectOption::new("a", "Alpha"),
        crate::SelectOption::new("b", "Beta"),
    ];
    let select = page
        .context
        .create_detached_component(document(), select)
        .unwrap();
    let field = select.stable_id();
    let mut queue = MutationQueue::new();
    queue.insert(page.rows[6][0], field, None);
    queue.set_scroll_offset(page.list, ScrollOffset { x: 0.0, y: 30.37 });
    let mut retained = snapshot(&page.context);
    edit(&mut page, &mut retained, queue, "placed");
    let pinned = |page: &Page| {
        page.context
            .world()
            .nodes
            .space_pins()
            .any(|id| id == field)
    };
    let mut height = 20.0;
    for (opened, at) in [(false, "closed"), (true, "open"), (false, "closed again")] {
        if opened != retained[&field].expanded.unwrap_or(false) {
            page.context.toggle_select(select).unwrap();
            edit(
                &mut page,
                &mut retained,
                MutationQueue::new(),
                &format!("{at}: toggled"),
            );
        }
        assert_eq!(pinned(&page), opened, "{at}: only an open select pins");
        for step in 0..3 {
            height += 1.37;
            let mut queue = MutationQueue::new();
            row_height(&mut queue, page.rows[0][0], height);
            let updated = edit(
                &mut page,
                &mut retained,
                queue,
                &format!("{at}, step {step}"),
            );
            assert_eq!(
                updated.contains(&field),
                opened,
                "{at}, step {step}: a select projects again under a move only while open"
            );
        }
    }
}

/// A box that clips, resized where it stands as the rows in it grow, cuts
/// what it holds by a new box: its whole subtree projects again, though it
/// did not move. (A scroller whose content grew keeps its box, and cuts
/// as before: see the scrolled list above.)
#[test]
fn a_clip_resized_in_place_projects_what_it_holds_again() {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let list = b.element(
        root,
        LayoutStyle {
            overflow_x: OverflowSpec::Hidden,
            overflow_y: OverflowSpec::Hidden,
            ..column(Some(300.0))
        },
    );
    let rows = uneven_rows(&mut b, list, 6);
    let mut page = laid_out(b, rows, list);
    let mut retained = snapshot(&page.context);
    let mut height = 20.0;
    for step in 0..4 {
        height += 1.37;
        let at = format!("step {step}");
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        let updated = edit(&mut page, &mut retained, queue, &at);
        for [_, cell, leaf] in &page.rows {
            assert!(
                updated.contains(cell) && updated.contains(leaf),
                "{at}: what a resized clip holds is cut by its new box"
            );
        }
    }
}

/// A rotation, a skew or a flip by `angle` (radians, for the turns) with a
/// move, as a CSS `matrix`.
fn turn(kind: &str, angle: f32) -> PaintTransform {
    let (sin, cos) = angle.sin_cos();
    match kind {
        "rotated" => PaintTransform {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 2.0,
            f: 1.0,
        },
        "skewed" => PaintTransform {
            c: angle.tan(),
            e: 1.5,
            ..Default::default()
        },
        "flipped" => PaintTransform {
            a: -1.0,
            f: 3.25,
            ..Default::default()
        },
        "scaled" => PaintTransform {
            a: 1.5,
            d: 1.25,
            e: 3.3,
            f: 7.1,
            ..Default::default()
        },
        other => panic!("no turn {other}"),
    }
}

/// Rows in a column under nested wrappers, the outermost first, each with
/// its transform; the innermost holds the column. Every cell of a row past
/// the first holds a sticky and a fixed box too when `hanging`, and an
/// absolute one measured from the row, each with a plain box inside.
fn turned_page(transforms: &[PaintTransform], hanging: bool) -> (Page, Vec<StableNodeId>) {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let mut parent = root;
    let mut wrappers = Vec::new();
    for (depth, &transform) in transforms.iter().enumerate() {
        let side = 10.0 * depth as f32;
        let mut wrapper_style = LayoutStyle {
            padding_left: Some(LengthSpec::Px(1.3)),
            padding_top: Some(LengthSpec::Px(2.1)),
            ..fixed(340.0 - side, 520.0 - side)
        };
        wrapper_style.transforms.transform = Some(transform);
        parent = b.element(parent, wrapper_style);
        wrappers.push(parent);
    }
    let list = b.element(parent, column(Some(300.0)));
    let rows = uneven_rows(&mut b, list, 12);
    if hanging {
        for [_, cell, _] in &rows[1..4] {
            for position in [
                PositionSpec::Sticky,
                PositionSpec::Fixed,
                PositionSpec::Absolute,
            ] {
                let held = b.element(
                    *cell,
                    LayoutStyle {
                        position,
                        offset_top: Some(LengthSpec::Px(1.0)),
                        offset_left: Some(LengthSpec::Px(7.0)),
                        ..fixed(12.0, 6.0)
                    },
                );
                b.element(held, fixed(4.0, 3.0));
            }
        }
    }
    (laid_out(b, rows, list), wrappers)
}

/// Rows under ancestors that rotate, skew, flip or nest a turn and a scale,
/// moved again and again by an uneven step. The innermost turn or scale is
/// handed to the platform as its box's own transform, and every box under
/// it is placed by its local offset inside it, so the moved rows project
/// alone and what they hold keeps its bounds to the bit.
#[test]
fn boxes_moved_under_a_turned_ancestor_keep_a_cold_projection_to_the_bit() {
    for (name, transforms, unit) in [
        ("rotated", vec![turn("rotated", 0.3)], [1.0, 1.0]),
        ("skewed", vec![turn("skewed", 0.4)], [1.0, 1.0]),
        ("flipped", vec![turn("flipped", 0.0)], [1.0, 1.0]),
        (
            "rotated twice",
            vec![turn("rotated", 0.3), turn("rotated", -0.7)],
            [1.0, 1.0],
        ),
        (
            "scaled in a rotation",
            vec![turn("rotated", 0.3), turn("scaled", 0.0)],
            [1.0, 1.0],
        ),
        (
            "rotated in a scale",
            vec![turn("scaled", 0.0), turn("rotated", 0.3)],
            [1.0, 1.0],
        ),
    ] {
        let (mut page, wrappers) = turned_page(&transforms, false);
        let mut retained = snapshot(&page.context);
        for id in &wrappers {
            assert!(
                retained[id].transform.is_some(),
                "{name}: {id:?} hands its turn or scale"
            );
        }
        grow_unevenly(
            &mut page,
            &mut retained,
            12,
            name,
            |page, retained, updated, at| {
                for id in &wrappers {
                    assert!(!updated.contains(id), "{at}: a wrapper did not move");
                }
                for [_, cell, leaf] in &page.rows[1..] {
                    for id in [cell, leaf] {
                        let local = page.context.world().local_layout_box(*id).unwrap();
                        let node = &retained[id];
                        assert_eq!(node.transform, None, "{at}: an upright box");
                        assert_eq!(
                            [
                                node.bounds.x,
                                node.bounds.y,
                                node.bounds.width,
                                node.bounds.height
                            ]
                            .map(f32::to_bits),
                            [
                                unit[0] * local.x,
                                unit[1] * local.y,
                                unit[0] * local.width,
                                unit[1] * local.height,
                            ]
                            .map(f32::to_bits),
                            "{at}: {id:?} is handed its offset in its turned space"
                        );
                    }
                }
            },
        );
    }
}

/// Boxes placed from their viewport boxes under a turn -- a sticky, a fixed
/// and an absolute box measured from the row, each holding a plain one --
/// are handed the map from the turned space to their own frames, found
/// from the pins when their rows move, and match a full projection; what
/// the moved rows hold otherwise does not project again. The sticky and the
/// absolute box turn with the row: each hands its own frame, and what it
/// holds is placed by its offset inside it, so it keeps its bounds.
#[test]
fn boxes_placed_from_the_viewport_under_a_turn_match_a_cold_projection() {
    for kind in ["rotated", "skewed", "flipped"] {
        let (mut page, _) = turned_page(&[turn(kind, 0.3)], true);
        let mut retained = snapshot(&page.context);
        let pinned: Vec<StableNodeId> = page.rows[1..4]
            .iter()
            .flat_map(|[_, cell, _]| {
                page.context
                    .world()
                    .nodes
                    .get(*cell)
                    .unwrap()
                    .hierarchy
                    .children
                    .iter()
                    .copied()
                    .skip(1)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(pinned.len(), 9);
        for id in &pinned {
            assert!(
                retained[id].transform.is_some(),
                "{kind}: {id:?} is handed the inverse turn"
            );
        }
        let held_by_absolute: Vec<StableNodeId> = pinned
            .iter()
            .skip(2)
            .step_by(3)
            .map(|id| {
                page.context
                    .world()
                    .nodes
                    .get(*id)
                    .unwrap()
                    .hierarchy
                    .children[0]
            })
            .collect();
        grow_unevenly(&mut page, &mut retained, 6, kind, |_, _, updated, at| {
            for id in &pinned {
                assert!(updated.contains(id), "{at}: {id:?} is placed from the page");
            }
            for id in &held_by_absolute {
                assert!(
                    !updated.contains(id),
                    "{at}: {id:?} is placed by its offset in its holder's frame"
                );
            }
        });
    }
}

/// A turned wrapper whose turn changes in the frame its rows move, then
/// again on its own: the wrapper hands its new turn, and every box matches
/// a full projection.
#[test]
fn a_turn_changed_while_rows_move_matches_a_cold_projection() {
    let (mut page, wrappers) = turned_page(&[turn("rotated", 0.3)], false);
    let wrapper = wrappers[0];
    let mut retained = snapshot(&page.context);
    let mut height = 20.0;
    for (step, angle) in [0.45_f32, 0.45, -0.2, -0.2, 1.1].into_iter().enumerate() {
        let at = format!("step {step}");
        let before = retained[&wrapper].transform;
        let mut queue = MutationQueue::new();
        let mut wrapper_style = LayoutStyle {
            padding_left: Some(LengthSpec::Px(1.3)),
            padding_top: Some(LengthSpec::Px(2.1)),
            ..fixed(340.0, 520.0)
        };
        wrapper_style.transforms.transform = Some(turn("rotated", angle));
        queue.set_style(wrapper, styled(wrapper_style));
        if step % 2 == 0 {
            height += 1.37;
            row_height(&mut queue, page.rows[0][0], height);
        }
        edit(&mut page, &mut retained, queue, &at);
        let (sin, cos) = angle.sin_cos();
        let handed = retained[&wrapper].transform.unwrap();
        assert_eq!(
            (handed.a, handed.b, handed.c, handed.d),
            (cos, sin, -sin, cos),
            "{at}: the wrapper hands its new turn"
        );
        if step == 0 {
            assert_ne!(before, Some(handed), "{at}: the turn changed");
        }
        // Then a move alone, under the new turn.
        height += 1.37;
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        let updated = edit(&mut page, &mut retained, queue, &format!("{at}, moved"));
        for [_, cell, leaf] in &page.rows[1..] {
            assert!(
                !updated.contains(cell) && !updated.contains(leaf),
                "{at}: what a moved row holds moved with it"
            );
        }
    }
}

/// A select in a row under a rotation: closed it moves with its row; open,
/// its list box and options stand in the field's turned space, upright
/// there, and project again under every move, as the menu reads the page.
#[test]
fn a_select_under_a_turn_moves_with_its_row_and_an_open_one_projects_again() {
    let (mut page, _) = turned_page(&[turn("rotated", 0.3)], false);
    let mut select = crate::Select::new(Some("a"));
    select.options = vec![
        crate::SelectOption::new("a", "Alpha"),
        crate::SelectOption::new("b", "Beta"),
    ];
    let select = page
        .context
        .create_detached_component(document(), select)
        .unwrap();
    let field = select.stable_id();
    let mut queue = MutationQueue::new();
    queue.insert(page.rows[6][0], field, None);
    let mut retained = snapshot(&page.context);
    edit(&mut page, &mut retained, queue, "placed");
    let mut height = 20.0;
    for (opened, at) in [(false, "closed"), (true, "open"), (false, "closed again")] {
        if opened != retained[&field].expanded.unwrap_or(false) {
            page.context.toggle_select(select).unwrap();
            edit(
                &mut page,
                &mut retained,
                MutationQueue::new(),
                &format!("{at}: toggled"),
            );
        }
        let listbox = crate::virtual_listbox_id(field).unwrap();
        assert_eq!(
            retained.contains_key(&listbox),
            opened,
            "{at}: its list box"
        );
        if opened {
            assert_eq!(
                retained[&listbox].transform, None,
                "{at}: upright in the field"
            );
            assert_matches_viewport(&page.context, &retained, at);
        }
        for step in 0..3 {
            height += 1.37;
            let mut queue = MutationQueue::new();
            row_height(&mut queue, page.rows[0][0], height);
            let updated = edit(
                &mut page,
                &mut retained,
                queue,
                &format!("{at}, step {step}"),
            );
            assert_eq!(
                updated.contains(&field),
                opened,
                "{at}, step {step}: a select projects again under a move only while open"
            );
        }
    }
}

/// Every node's box, placed through the transforms its chain hands
/// ([`crate::AccessibilityPlacements`]), stands where a viewport projection
/// puts it, up to rounding.
fn assert_matches_viewport(
    context: &AppContext,
    retained: &BTreeMap<StableNodeId, AccessibilityNode>,
    at: &str,
) {
    let placements = crate::AccessibilityPlacements::new(retained.values());
    for node in context
        .world()
        .project_accessibility_in_viewport(document())
    {
        let placed = placements.viewport_bounds(node.id).unwrap();
        let want = node.bounds;
        assert!(
            [
                placed.x - want.x,
                placed.y - want.y,
                placed.width - want.width,
                placed.height - want.height
            ]
            .iter()
            .all(|delta| delta.abs() <= 2e-3),
            "{at}: {:?} placed at {placed:?}, in the viewport at {want:?}",
            node.id
        );
    }
}

/// The corners the transforms a relative projection hands put each box's
/// own box at are the corners the old projection took the box around:
/// the layout box's corners through every transform and scroll above it --
/// for boxes placed by their offsets and for those placed from the viewport
/// that hand their own frames alike. Every box's box around is the viewport
/// projection's.
#[test]
fn placed_corners_are_the_turned_corners_of_a_cold_projection() {
    for (name, transforms) in [
        ("rotated", vec![turn("rotated", 0.3)]),
        ("skewed", vec![turn("skewed", 0.4)]),
        ("flipped", vec![turn("flipped", 0.0)]),
        (
            "nested",
            vec![
                turn("scaled", 0.0),
                turn("rotated", 0.3),
                turn("scaled", 0.0),
                turn("skewed", -0.2),
            ],
        ),
    ] {
        let (mut page, _) = turned_page(&transforms, true);
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], 21.37);
        let mut retained = snapshot(&page.context);
        edit(&mut page, &mut retained, queue, name);
        let framed = assert_turned_corners(&page.context, &retained, name);
        assert_eq!(
            framed, 9,
            "{name}: the sticky, fixed and absolute boxes under the turn hand their own frames"
        );
    }
}

/// A row that turns about its centre, resized where it stands: its turn
/// moves with the centre, so its space moves as a moved box's does. What it
/// holds by offsets keeps its bounds; a fixed box two levels down, placed
/// from the viewport, projects again.
#[test]
fn a_turned_box_resized_in_place_moves_its_space() {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let list = b.element(root, column(Some(300.0)));
    let rows = uneven_rows(&mut b, list, 3);
    let turned = rows[0][0];
    let cell = rows[0][1];
    let leaf = rows[0][2];
    let fixed_box = b.element(
        cell,
        LayoutStyle {
            position: PositionSpec::Fixed,
            offset_top: Some(LengthSpec::Px(5.0)),
            offset_left: Some(LengthSpec::Px(9.0)),
            ..fixed(12.0, 6.0)
        },
    );
    let mut page = laid_out(b, rows, list);
    let mut retained = snapshot(&page.context);
    let mut height = 20.0;
    for step in 0..6 {
        height += 1.37;
        let at = format!("step {step}");
        let mut queue = MutationQueue::new();
        let mut turned_style = LayoutStyle {
            flex_shrink: Some(0.0),
            position: PositionSpec::Relative,
            ..fixed(300.0, height)
        };
        turned_style.transforms.transform = Some(turn("rotated", 0.3));
        queue.set_style(turned, styled(turned_style));
        let updated = edit(&mut page, &mut retained, queue, &at);
        assert!(updated.contains(&turned), "{at}: its turn moved");
        assert!(
            retained[&turned].transform.is_some(),
            "{at}: it hands its turn"
        );
        if step > 0 {
            assert!(
                !updated.contains(&cell) && !updated.contains(&leaf),
                "{at}: what it holds kept its offsets in it"
            );
            assert!(
                updated.contains(&fixed_box),
                "{at}: placed from the viewport"
            );
        }
    }
}

/// What [`viewport_page`] holds.
struct ViewportPage {
    page: Page,
    /// The page box: a root of the document of its own, placed from the
    /// viewport, relative, so the escaping box is measured from it.
    root: StableNodeId,
    /// A sticky, a fixed and an absolute box measured from the root that
    /// escapes the scroller's clip, each with a plain box inside.
    placed: [StableNodeId; 3],
    /// The scroller between the rows.
    scroller: StableNodeId,
}

fn placed_style(position: PositionSpec, transform: Option<PaintTransform>) -> LayoutStyle {
    let mut layout = LayoutStyle {
        position,
        offset_top: Some(LengthSpec::Px(1.0)),
        offset_left: Some(LengthSpec::Px(7.0)),
        ..fixed(12.0, 6.0)
    };
    if transform.is_some() {
        layout.transforms.transform = transform;
    }
    layout
}

fn root_style(transform: Option<PaintTransform>) -> LayoutStyle {
    let mut layout = LayoutStyle {
        position: PositionSpec::Relative,
        padding_left: Some(LengthSpec::Px(1.3)),
        padding_top: Some(LengthSpec::Px(2.1)),
        ..column(Some(360.0))
    };
    if transform.is_some() {
        layout.transforms.transform = transform;
    }
    layout
}

/// A root page box of its own (no document node above it, as an
/// application's page is) holding six rows, then a scroller of eight plain
/// rows. The second row's cell holds a sticky and a fixed box, each turned,
/// and the scroller's last row an absolute box, turned too, measured from
/// the root, which escapes the scroller's clip (#201); each holds a plain
/// box.
fn viewport_page(root_turn: Option<PaintTransform>) -> ViewportPage {
    let (mut b, _) = Builder::new(document(), 1);
    let root = b.detached(root_style(root_turn));
    let rows = uneven_rows(&mut b, root, 6);
    let scroller = b.element(
        root,
        LayoutStyle {
            overflow_x: OverflowSpec::Auto,
            overflow_y: OverflowSpec::Auto,
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            ..fixed(300.0, 60.0)
        },
    );
    let mut last = None;
    for _ in 0..8 {
        let row = b.element(
            scroller,
            LayoutStyle {
                flex_shrink: Some(0.0),
                ..fixed(300.0, 20.0)
            },
        );
        let cell = b.element(row, fixed(100.0, 14.0));
        b.element(cell, fixed(50.0, 5.0));
        last = Some(cell);
    }
    let cell = rows[1][1];
    let hold = |b: &mut Builder, parent, position, transform| {
        let held = b.element(parent, placed_style(position, Some(transform)));
        b.element(held, fixed(4.0, 3.0));
        held
    };
    let placed = [
        hold(&mut b, cell, PositionSpec::Sticky, turn("rotated", 0.25)),
        hold(&mut b, cell, PositionSpec::Fixed, turn("skewed", 0.3)),
        hold(
            &mut b,
            last.unwrap(),
            PositionSpec::Absolute,
            turn("rotated", -0.4),
        ),
    ];
    ViewportPage {
        page: laid_out(b, rows, scroller),
        root,
        placed,
        scroller,
    }
}

/// `id` and everything under it.
fn subtree(context: &AppContext, id: StableNodeId) -> Vec<StableNodeId> {
    let mut out = Vec::new();
    let mut pending = vec![id];
    while let Some(id) = pending.pop() {
        out.push(id);
        if let Some(node) = context.world().nodes.get(id) {
            pending.extend(node.hierarchy.children.iter().copied());
        }
    }
    out
}

/// Every box placed by its offset or handing its own frame stands, through
/// the transforms the relative projection hands ([`crate::AccessibilityPlacements`]),
/// where its layout box's corners stand through every transform and scroll
/// above it; every box's box around matches a viewport projection. Returns
/// how many framed boxes placed from the viewport were compared.
fn assert_turned_corners(
    context: &AppContext,
    retained: &BTreeMap<StableNodeId, AccessibilityNode>,
    at: &str,
) -> usize {
    let world = context.world();
    let placements = crate::AccessibilityPlacements::new(retained.values());
    let mut memo = super::ProjectionMemo::default();
    let mut framed = 0;
    for (id, node) in retained {
        let Some(space) = world.accessible_space(*id, &mut memo) else {
            continue;
        };
        if !space.local && !space.framed {
            continue;
        }
        if !space.local {
            framed += 1;
        }
        world
            .accessibility_ancestor_transform(*id, &mut memo)
            .unwrap();
        let ([a, b, c, d, e, f], _) = memo.transforms[id].own;
        let page_box = world.component_layout_box(*id).unwrap();
        let turned = |x: f32, y: f32| [a * x + c * y + e, b * x + d * y + f];
        let (x, y, w, h) = (page_box.x, page_box.y, page_box.width, page_box.height);
        let want = [
            turned(x, y),
            turned(x + w, y),
            turned(x + w, y + h),
            turned(x, y + h),
        ];
        let got = placements.corners(*id).unwrap();
        assert!(
            got.iter()
                .flatten()
                .zip(want.iter().flatten())
                .all(|(got, want)| (got - want).abs() <= 2e-3),
            "{at}: {id:?} ({:?}) placed at {got:?}, turned to {want:?}",
            node.transform
        );
    }
    assert_matches_viewport(context, retained, at);
    framed
}

/// A root page box, a fixed, a sticky and an escaping absolute box -- all
/// placed from the viewport -- turned again and again, with the rows moved
/// by an uneven step and the scroller scrolled between and in the same
/// frames. Each hands its own frame, so a turn of it projects it and the
/// pins under it, and what it holds by offsets keeps its bounds to the bit.
#[test]
fn boxes_placed_from_the_viewport_that_turn_keep_what_they_hold() {
    let ViewportPage {
        mut page,
        root,
        placed,
        scroller,
    } = viewport_page(Some(turn("rotated", 0.1)));
    let mut retained = snapshot(&page.context);
    assert!(
        assert_turned_corners(&page.context, &retained, "built") >= 4,
        "the root and the three placed boxes hand their own frames"
    );
    for id in std::iter::once(&root).chain(&placed) {
        assert!(retained[id].transform.is_some(), "{id:?} hands its frame");
    }
    // What a turn of the root projects: it, and the pins under it -- the
    // placed boxes and the scroller, whose clip in viewport coordinates
    // cuts what it holds -- with what they hold.
    let mut under_pins = vec![root];
    for id in placed.iter().chain([&scroller]) {
        under_pins.extend(subtree(&page.context, *id));
    }
    let mut height = 20.0;
    let mut scroll = 0.0;
    for step in 0..10 {
        let angle = 0.1 + 0.137 * (step + 1) as f32;
        let at = format!("step {step}");
        // The root turned on its own.
        let mut queue = MutationQueue::new();
        queue.set_style(root, styled(root_style(Some(turn("rotated", angle)))));
        let updated = edit(&mut page, &mut retained, queue, &format!("{at}: root"));
        assert!(updated.contains(&root), "{at}: the root projects again");
        assert!(
            updated.iter().all(|id| under_pins.contains(id)),
            "{at}: a turn of the root projected {updated:?}"
        );
        // Each placed box turned on its own: it alone projects again.
        for (index, (&id, position)) in placed
            .iter()
            .zip([
                PositionSpec::Sticky,
                PositionSpec::Fixed,
                PositionSpec::Absolute,
            ])
            .enumerate()
        {
            let mut queue = MutationQueue::new();
            queue.set_style(
                id,
                styled(placed_style(
                    position,
                    Some(turn(
                        ["rotated", "skewed", "flipped"][(index + step) % 3],
                        angle * 0.7,
                    )),
                )),
            );
            let updated = edit(&mut page, &mut retained, queue, &format!("{at}: {id:?}"));
            assert_eq!(
                updated,
                vec![id],
                "{at}: a turn of {id:?} projects it alone"
            );
        }
        // A move by an uneven step, alone and with a turn.
        height += 1.37;
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        let updated = edit(&mut page, &mut retained, queue, &format!("{at}: moved"));
        for [_, cell, leaf] in &page.rows[1..] {
            assert!(
                !updated.contains(cell) && !updated.contains(leaf),
                "{at}: what a moved row holds moved with it"
            );
        }
        // The sticky box's offset is worked out again whenever its rows
        // move, which marks what it holds, and the escaping box is in the
        // moved scroller, whose clip moved: the fixed box alone stays.
        for id in &placed[1..2] {
            let held = page
                .context
                .world()
                .nodes
                .get(*id)
                .unwrap()
                .hierarchy
                .children[0];
            assert!(
                !updated.contains(&held),
                "{at}: {held:?} keeps its offset in its holder's frame"
            );
        }
        height += 1.37;
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        queue.set_style(root, styled(root_style(Some(turn("skewed", angle * 0.5)))));
        edit(
            &mut page,
            &mut retained,
            queue,
            &format!("{at}: moved and turned"),
        );
        // A scroll between, and with a turn of the escaping box.
        scroll += 3.37;
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(scroller, ScrollOffset { x: 0.0, y: scroll });
        edit(&mut page, &mut retained, queue, &format!("{at}: scrolled"));
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(
            scroller,
            ScrollOffset {
                x: 0.0,
                y: scroll * 0.5,
            },
        );
        queue.set_style(
            placed[2],
            styled(placed_style(
                PositionSpec::Absolute,
                Some(turn("rotated", -angle)),
            )),
        );
        edit(
            &mut page,
            &mut retained,
            queue,
            &format!("{at}: scrolled and turned"),
        );
        assert!(assert_turned_corners(&page.context, &retained, &at) >= 4);
    }
    // The root goes upright, then turns again: its space changes, and all
    // it holds projects again.
    for (transform, at) in [(None, "upright"), (Some(turn("flipped", 0.0)), "flipped")] {
        let mut queue = MutationQueue::new();
        queue.set_style(root, styled(root_style(transform)));
        let updated = edit(&mut page, &mut retained, queue, at);
        assert!(
            updated.contains(&page.rows[3][2]),
            "{at}: the space changed under the rows"
        );
        assert_eq!(
            retained[&root].transform.is_some(),
            transform.is_some(),
            "{at}"
        );
        assert_turned_corners(&page.context, &retained, at);
    }
}

/// An upright scale `(a, d)`, with a move, as a CSS `matrix`.
fn scale(a: f32, d: f32) -> PaintTransform {
    PaintTransform {
        a,
        d,
        e: 2.25,
        f: -1.5,
        ..Default::default()
    }
}

/// Rows in a column under a scaled wrapper, holding a scaled box of rows of
/// its own: each hands `diag(a, d)` and what it holds is placed by its
/// local offsets inside it, so changing a factor -- larger, smaller, one
/// axis only, all but zero -- projects that box alone, and moves under
/// either keep what the moved rows hold. A scale of one on both axes only
/// moves: the box hands no transform, and going to it or from it changes
/// the space, so what it holds projects again.
#[test]
fn a_scale_changed_again_and_again_projects_the_box_alone() {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let wrapper_style = |transform: Option<PaintTransform>| {
        let mut layout = LayoutStyle {
            padding_left: Some(LengthSpec::Px(1.3)),
            padding_top: Some(LengthSpec::Px(2.1)),
            ..fixed(340.0, 520.0)
        };
        if transform.is_some() {
            layout.transforms.transform = transform;
        }
        layout
    };
    let inner_style = |transform: Option<PaintTransform>| {
        let mut layout = LayoutStyle {
            padding_top: Some(LengthSpec::Px(0.7)),
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            ..fixed(300.0, 90.0)
        };
        if transform.is_some() {
            layout.transforms.transform = transform;
        }
        layout
    };
    let wrapper = b.element(root, wrapper_style(Some(scale(1.5, 1.25))));
    let list = b.element(wrapper, column(Some(300.0)));
    let rows = uneven_rows(&mut b, list, 8);
    let inner = b.element(list, inner_style(Some(scale(0.8, 1.1))));
    let inner_rows = uneven_rows(&mut b, inner, 3);
    // A sticky box under the scales: placed from the viewport in a scaled
    // space, it hands its own frame.
    let sticky = b.element(rows[2][1], placed_style(PositionSpec::Sticky, None));
    let held = b.element(sticky, fixed(4.0, 3.0));
    let _ = held;
    let mut page = laid_out(b, rows, list);
    let mut retained = snapshot(&page.context);
    let check_leaves =
        |page: &Page, retained: &BTreeMap<StableNodeId, AccessibilityNode>, at: &str| {
            for [_, cell, leaf] in page.rows[1..].iter().chain(&inner_rows) {
                for id in [cell, leaf] {
                    let local = page.context.world().local_layout_box(*id).unwrap();
                    let node = &retained[id];
                    assert_eq!(node.transform, None, "{at}: an upright box");
                    assert_eq!(
                        [
                            node.bounds.x,
                            node.bounds.y,
                            node.bounds.width,
                            node.bounds.height
                        ]
                        .map(f32::to_bits),
                        [local.x, local.y, local.width, local.height].map(f32::to_bits),
                        "{at}: {id:?} is handed its local offset in the scaled space"
                    );
                }
            }
        };
    check_leaves(&page, &retained, "built");
    let mut height = 20.0;
    let steps = [
        (Some(scale(1.37, 1.37)), Some(scale(0.8, 1.1)), true),
        (Some(scale(1.37, 1.37)), Some(scale(2.0, 0.5)), true),
        (Some(scale(0.5, 1.37)), Some(scale(2.0, 0.5)), true),
        (Some(scale(1e-3, 1e-3)), Some(scale(1e-3, 2.0)), true),
        (Some(scale(2.0, 1e-4)), Some(scale(0.9, 0.9)), true),
        (Some(scale(-1.0, 1.0)), Some(scale(0.9, 0.9)), true),
        (Some(scale(1.0, 1.0)), Some(scale(0.9, 0.9)), false),
        (Some(scale(1.25, 1.0)), Some(scale(1.0, 1.0)), false),
        (Some(scale(1.25, 1.0)), Some(scale(1.5, 1.5)), false),
        (Some(scale(1.25, 1.0)), Some(scale(1.75, 1.5)), true),
    ];
    let (mut outer_was, mut inner_was) = (Some(scale(1.5, 1.25)), Some(scale(0.8, 1.1)));
    for (step, (outer, inner_scale, kept)) in steps.into_iter().enumerate() {
        let at = format!("step {step}");
        let mut queue = MutationQueue::new();
        if outer != outer_was {
            queue.set_style(wrapper, styled(wrapper_style(outer)));
        }
        if inner_scale != inner_was {
            queue.set_style(inner, styled(inner_style(inner_scale)));
        }
        let changed: Vec<StableNodeId> = [
            (outer != outer_was, wrapper),
            (inner_scale != inner_was, inner),
        ]
        .into_iter()
        .filter_map(|(changed, id)| changed.then_some(id))
        .collect();
        (outer_was, inner_was) = (outer, inner_scale);
        let updated = edit(&mut page, &mut retained, queue, &at);
        if kept {
            // The changed boxes, and the sticky box placed from the
            // viewport under them.
            assert!(
                updated
                    .iter()
                    .all(|id| changed.contains(id) || *id == sticky || *id == held),
                "{at}: a factor change projected {updated:?}"
            );
            for id in &changed {
                assert!(updated.contains(id), "{at}: {id:?} projects again");
                let handed = retained[id].transform.expect("a scale hands its frame");
                if handed.a > 0.0 && handed.d > 0.0 {
                    assert_eq!(
                        (handed.b, handed.c, handed.e, handed.f),
                        (0.0, 0.0, 0.0, 0.0),
                        "{at}: an upright scale hands diag(a, d)"
                    );
                }
            }
        } else {
            assert!(
                updated.len() > changed.len() + 2,
                "{at}: a change to or from a scale of one changes the space"
            );
        }
        assert_eq!(
            retained[&wrapper].transform.is_some(),
            outer.is_some_and(|t| (t.a, t.d) != (1.0, 1.0)),
            "{at}: a scale of one hands no transform"
        );
        check_leaves(&page, &retained, &at);
        assert_turned_corners(&page.context, &retained, &at);
        // A move under the new factors: what the moved rows hold stays.
        height += 1.37;
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        let updated = edit(&mut page, &mut retained, queue, &format!("{at}: moved"));
        for [_, cell, leaf] in page.rows[1..].iter().chain(&inner_rows) {
            assert!(
                !updated.contains(cell) && !updated.contains(leaf),
                "{at}: what a moved row holds moved with it"
            );
        }
        check_leaves(&page, &retained, &format!("{at}: moved"));
    }
}

/// Children of boxes in a scrolled list, which the projection keeps or drops
/// by whether they have a viewport box, tested once per parent's map: a box
/// placed by its offset whose page box ends past the largest float under a
/// scaled box, a hidden box and a box that lays out nothing; and a box in a
/// header that sticks to the list's top. Across scrolls
/// and moves by an uneven step, every parent lists the children a full
/// projection does, and the overflowing box is not listed.
#[test]
fn children_under_a_scroll_chain_are_kept_as_their_viewport_boxes_say() {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let list = b.element(
        root,
        LayoutStyle {
            overflow_x: OverflowSpec::Auto,
            overflow_y: OverflowSpec::Auto,
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            ..fixed(300.0, 120.0)
        },
    );
    // A header that sticks to the list's top once it scrolls, holding a box:
    // the map under it moves with the sticky offset.
    let header = b.element(
        list,
        LayoutStyle {
            position: PositionSpec::Sticky,
            offset_top: Some(LengthSpec::Px(0.0)),
            flex_shrink: Some(0.0),
            ..fixed(300.0, 8.0)
        },
    );
    let held = b.element(header, fixed(30.0, 4.0));
    let rows = uneven_rows(&mut b, list, 8);
    let cell = rows[2][1];
    let mut scaled_style = LayoutStyle {
        direction: Some(FlexDirection::Column),
        ..fixed(20.0, 10.0)
    };
    scaled_style.transforms.transform = Some(scale(4.0, 4.0));
    let scaled = b.element(rows[3][1], scaled_style);
    let wide = b.element(scaled, fixed(3e38, 5.0));
    let beside = b.element(scaled, fixed(5.0, 5.0));
    let hidden = b.element(cell, fixed(10.0, 10.0));
    b.queue.set_accessibility(
        hidden,
        crate::AccessibilityState {
            hidden: true,
            ..Default::default()
        },
    );
    let gone = b.element(
        cell,
        LayoutStyle {
            display: Some(nana_ui_core::DisplaySpec::None),
            ..fixed(10.0, 10.0)
        },
    );
    let mut page = laid_out(b, rows, list);
    let mut retained = snapshot(&page.context);
    let check = |retained: &BTreeMap<StableNodeId, AccessibilityNode>, at: &str| {
        let listed = &retained[&cell].children;
        assert!(
            listed.contains(&page_leaf(&retained[&cell])),
            "{at}: the leaf stays"
        );
        for id in [hidden, gone] {
            assert!(
                !listed.contains(&id),
                "{at}: {id:?} has no viewport box to keep"
            );
        }
        assert_eq!(
            retained[&scaled].children,
            vec![beside],
            "{at}: a box ending past the largest float has no viewport box"
        );
        assert!(!retained.contains_key(&wide), "{at}: nor a node");
        assert_eq!(
            retained[&header].children,
            vec![held],
            "{at}: the header's box"
        );
    };
    check(&retained, "built");
    let mut scroll = 0.0;
    let mut height = 20.0;
    for step in 0..8 {
        let at = format!("step {step}");
        scroll += 7.37;
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(list, ScrollOffset { x: 0.0, y: scroll });
        edit(&mut page, &mut retained, queue, &format!("{at}: scrolled"));
        check(&retained, &at);
        height += 1.37;
        let mut queue = MutationQueue::new();
        row_height(&mut queue, page.rows[0][0], height);
        if step % 2 == 1 {
            queue.set_scroll_offset(
                list,
                ScrollOffset {
                    x: 0.0,
                    y: scroll * 0.5,
                },
            );
        }
        let updated = edit(&mut page, &mut retained, queue, &format!("{at}: moved"));
        check(&retained, &format!("{at}: moved"));
        assert!(
            updated.contains(&page.rows[2][0]),
            "{at}: a moved row projects again"
        );
    }
}

/// The first child of a cell node: its leaf.
fn page_leaf(cell: &AccessibilityNode) -> StableNodeId {
    cell.children[0]
}

/// Boxes placed from the viewport on a plain page whose own turn leaves
/// their box around where it was -- a sticky box flipped about its centre,
/// an absolute one in a cell measured from the row above it turned half
/// round -- hand their
/// own frames, placed in their parents' spaces through their page
/// positions: when their rows move by an uneven step they project again,
/// and match a cold projection to the bit.
#[test]
fn boxes_whose_turn_keeps_their_box_around_project_again_when_they_move() {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let list = b.element(root, column(Some(300.0)));
    let rows = uneven_rows(&mut b, list, 6);
    let half = std::f32::consts::PI;
    let placed = [
        b.element(
            rows[2][1],
            placed_style(
                PositionSpec::Sticky,
                Some(PaintTransform {
                    a: -1.0,
                    ..Default::default()
                }),
            ),
        ),
        b.element(
            rows[3][1],
            placed_style(
                PositionSpec::Absolute,
                Some(PaintTransform {
                    a: half.cos(),
                    b: half.sin(),
                    c: -half.sin(),
                    d: half.cos(),
                    ..Default::default()
                }),
            ),
        ),
    ];
    for id in placed {
        b.element(id, fixed(4.0, 3.0));
    }
    let mut page = laid_out(b, rows, list);
    let mut retained = snapshot(&page.context);
    for id in placed {
        assert!(retained[&id].transform.is_some(), "{id:?} hands its frame");
    }
    grow_unevenly(
        &mut page,
        &mut retained,
        12,
        "flipped",
        |page, retained, updated, at| {
            for id in placed {
                assert!(updated.contains(&id), "{at}: {id:?} projects again");
            }
            assert_turned_corners(&page.context, retained, at);
        },
    );
}

/// A scaled box under a 3D context: its parent's map to the viewport is not
/// one a box can be placed in, so the box hands no frame of its own -- its
/// space is the viewport's, scaled, and what it holds stands at scaled
/// offsets. A change of its factor changes that space: what it holds
/// projects again, and matches a cold projection.
#[test]
fn a_scale_under_a_3d_context_projects_what_it_holds_again() {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let context = b.element(
        root,
        LayoutStyle {
            transforms: nana_ui_core::TransformStyle {
                preserve_3d: true,
                ..Default::default()
            }
            .into(),
            direction: Some(FlexDirection::Column),
            ..fixed(340.0, 400.0)
        },
    );
    let scaled_style = |transform| LayoutStyle {
        transforms: nana_ui_core::TransformStyle {
            transform: Some(transform),
            ..Default::default()
        }
        .into(),
        direction: Some(FlexDirection::Column),
        padding_top: Some(LengthSpec::Px(0.7)),
        ..fixed(300.0, 200.0)
    };
    let scaled = b.element(context, scaled_style(scale(1.5, 1.25)));
    let list = b.element(scaled, column(Some(300.0)));
    let rows = uneven_rows(&mut b, list, 4);
    let mut page = laid_out(b, rows, list);
    let mut retained = snapshot(&page.context);
    for (step, factor) in [1.37_f32, 0.8, 2.0].into_iter().enumerate() {
        let at = format!("step {step}");
        let mut queue = MutationQueue::new();
        queue.set_style(scaled, styled(scaled_style(scale(factor, 1.25))));
        let updated = edit(&mut page, &mut retained, queue, &at);
        assert!(
            updated.contains(&page.rows[1][2]),
            "{at}: the scaled space changed under the rows"
        );
        let leaf = page.rows[1][2];
        let local = page.context.world().local_layout_box(leaf).unwrap();
        assert_eq!(
            retained[&leaf].bounds.width.to_bits(),
            (factor * local.width).to_bits(),
            "{at}: a leaf stands at its scaled offset"
        );
    }
    // The column under it goes from no transform to a scale and back: in
    // the scaled space above, a scale of its own is a frame of its own, so
    // what it holds stands at its own offsets, then at scaled ones again.
    for (step, transform) in [Some(scale(1.1, 1.1)), None, Some(scale(0.9, 1.2))]
        .into_iter()
        .enumerate()
    {
        let at = format!("column, step {step}");
        let mut queue = MutationQueue::new();
        queue.set_style(
            list,
            styled(LayoutStyle {
                transforms: nana_ui_core::TransformStyle {
                    transform,
                    ..Default::default()
                }
                .into(),
                ..column(Some(300.0))
            }),
        );
        let updated = edit(&mut page, &mut retained, queue, &at);
        assert!(
            updated.contains(&page.rows[1][2]),
            "{at}: the column's space changed under the rows"
        );
    }
}

/// A list scrolled by uneven steps, again and again, holding a sticky header
/// with a box in it, rows of uneven boxes, a fixed and an absolute box in a
/// row and a nested scroller in another. A scroll is the list's own: the
/// delta projects the list (its scroll), the pins under it -- the sticky
/// header, the fixed box, the absolute box placed from its viewport box --
/// and what the nested scroller holds (its clip moved on the viewport), and
/// leaves every row and what it holds as it was, which is a cold projection's to the bit. The
/// tree placed through the scrolls stands where the viewport projection
/// puts it, and a frame that scrolls both scrollers, or scrolls and moves,
/// matches a full projection too, as does a scroll back to the origin.
#[test]
fn a_scroll_projects_the_scroller_and_its_pins_alone() {
    let (mut b, root) = Builder::page(document(), 1, 400.0);
    let list = b.element(
        root,
        LayoutStyle {
            overflow_x: OverflowSpec::Auto,
            overflow_y: OverflowSpec::Auto,
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            ..fixed(300.0, 120.0)
        },
    );
    let header = b.element(
        list,
        LayoutStyle {
            position: PositionSpec::Sticky,
            offset_top: Some(LengthSpec::Px(0.0)),
            flex_shrink: Some(0.0),
            ..fixed(300.0, 8.0)
        },
    );
    let held = b.element(header, fixed(30.0, 4.0));
    let rows = uneven_rows(&mut b, list, 16);
    let placed = |position, left: f32, top: f32| LayoutStyle {
        position,
        offset_left: Some(LengthSpec::Px(left)),
        offset_top: Some(LengthSpec::Px(top)),
        ..fixed(10.0, 10.0)
    };
    let fixed_box = b.element(rows[5][1], placed(PositionSpec::Fixed, 320.0, 10.0));
    let absolute = b.element(rows[5][1], placed(PositionSpec::Absolute, 60.0, 2.0));
    let inner = b.element(
        rows[7][1],
        LayoutStyle {
            overflow_x: OverflowSpec::Auto,
            overflow_y: OverflowSpec::Auto,
            direction: Some(FlexDirection::Column),
            ..fixed(60.0, 10.0)
        },
    );
    let inner_rows = (0..3)
        .map(|_| {
            b.element(
                inner,
                LayoutStyle {
                    flex_shrink: Some(0.0),
                    ..fixed(60.0, 7.3)
                },
            )
        })
        .collect::<Vec<_>>();
    let mut page = laid_out(b, rows, list);
    let mut retained = snapshot(&page.context);
    let carried = page
        .rows
        .iter()
        .flatten()
        .copied()
        .chain([held])
        .collect::<Vec<_>>();
    let mut scroll = 0.0;
    for step in 0..6 {
        scroll += 13.37;
        let at = format!("step {step}");
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(list, ScrollOffset { x: 0.0, y: scroll });
        let updated = edit(&mut page, &mut retained, queue, &format!("{at}: scrolled"));
        assert!(updated.contains(&list), "{at}: the list projects again");
        assert_eq!(retained[&list].scroll, [0.0, scroll], "{at}: its scroll");
        for id in &carried {
            assert!(
                !updated.contains(id),
                "{at}: {id:?} scrolled with the list and keeps its bounds"
            );
        }
        let mut expected = vec![list, header, fixed_box, absolute, inner];
        expected.extend(&inner_rows);
        expected.sort_unstable();
        let mut got = updated.clone();
        got.sort_unstable();
        assert_eq!(got, expected, "{at}: the scroller and its pins alone");
        assert_matches_viewport(&page.context, &retained, &format!("{at}: scrolled"));
        // Both scrollers in one frame, then a scroll and a move.
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(
            list,
            ScrollOffset {
                x: 0.0,
                y: scroll + 0.5,
            },
        );
        queue.set_scroll_offset(
            inner,
            ScrollOffset {
                x: 0.0,
                y: 1.1 * (step % 3) as f32,
            },
        );
        edit(&mut page, &mut retained, queue, &format!("{at}: both"));
        assert_matches_viewport(&page.context, &retained, &format!("{at}: both"));
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(list, ScrollOffset { x: 0.0, y: scroll });
        row_height(&mut queue, page.rows[0][0], 20.0 + 1.37 * (step + 1) as f32);
        edit(
            &mut page,
            &mut retained,
            queue,
            &format!("{at}: scrolled and moved"),
        );
        assert_matches_viewport(&page.context, &retained, &format!("{at}: moved"));
    }
    // Back to the origin: no scroll offset is left to tell the stuck header
    // by, and it unsticks all the same.
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(list, ScrollOffset::default());
    let updated = edit(&mut page, &mut retained, queue, "back to the origin");
    assert!(updated.contains(&header), "the header unsticks");
    assert_eq!(retained[&list].scroll, [0.0, 0.0], "the list's scroll");
    assert_matches_viewport(&page.context, &retained, "back to the origin");
}
