//! A live, flat drag shows itself on the presentation layer: the carried row
//! follows the pointer and the rows between make room.

use super::*;
use crate::framework::AppContext;
use crate::{AnimatableProperty, DocumentId, LayoutViewport, MotionValue};
use std::time::Duration;

const ROW: f32 = 40.0;
const GAP: f32 = 4.0;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

/// A live list of four 40px rows 4px apart, each holding one child.
struct LiveRows {
    context: AppContext,
    list: crate::Entity<ReorderList>,
    rows: Vec<StableNodeId>,
    inner: Vec<StableNodeId>,
    boxes: Vec<LayoutBox>,
    events: Arc<std::sync::Mutex<Vec<ReorderListEvent>>>,
}

impl LiveRows {
    fn new(tree_drop: bool) -> Self {
        let mut context = AppContext::new();
        let list = context
            .create_component(
                document(),
                ReorderList::new(["a", "b", "c", "d"].map(|value| ReorderItem::new(value, value)))
                    .live_rows(true)
                    .tree_drop(tree_drop)
                    .spacing(GAP),
            )
            .unwrap();
        let mut rows = Vec::new();
        let mut inner = Vec::new();
        for _ in 0..4 {
            let row = context
                .create_detached_component(
                    document(),
                    crate::Stack::column(0.0).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Fill);
                        layout.height = Some(LengthSpec::Px(ROW));
                    }),
                )
                .unwrap();
            context.append_child(list, row).unwrap();
            let child = context
                .create_detached_component(
                    document(),
                    crate::Stack::column(0.0).height(LengthSpec::Px(20.0)),
                )
                .unwrap();
            context.append_child(row, child).unwrap();
            rows.push(row.stable_id());
            inner.push(child.stable_id());
        }
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        context
            .on(list, move |_, event: &ReorderListEvent, _| {
                observed.lock().unwrap().push(event.clone());
            })
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(200.0, 400.0))
            .unwrap();
        let boxes = rows
            .iter()
            .map(|row| context.world().layout_box(*row).unwrap())
            .collect();
        let _ = context.take_system_work();
        Self {
            context,
            list,
            rows,
            inner,
            boxes,
            events,
        }
    }

    fn press(&mut self, row: usize) -> (f32, f32) {
        let point = (self.boxes[row].x + 20.0, self.boxes[row].y + 20.0);
        assert!(
            self.context
                .begin_reorder_list_pointer(document(), 1, self.list.stable_id(), point.0, point.1)
                .unwrap()
        );
        point
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.context
            .update_reorder_list_pointer(document(), 1, x, y)
            .unwrap();
    }

    /// Row `index`'s presented transform now, or `None` with nothing over
    /// its logical one.
    fn presented(&self, index: usize) -> Option<nana_ui_core::PaintTransform> {
        let world = self.context.world();
        let pair = world.presentation_pair(
            self.rows[index],
            AnimatableProperty::Transform,
            world.animation_now(),
        )?;
        match pair.presentation {
            MotionValue::Transform(transform) => Some(transform),
            _ => None,
        }
    }

    /// Where row `index`'s presentation is heading, `[dx, dy]`.
    fn heading(&self, index: usize) -> Option<[f32; 2]> {
        let world = self.context.world();
        let target = crate::MotionTargetId::new(self.rows[index].get())?;
        let mut heading = None;
        for overlay in world.presentation_store().overlays() {
            if overlay.track.target == target
                && let crate::MotionTo::Value(MotionValue::Transform(to)) = overlay.track.to
            {
                heading = Some([to.e, to.f]);
            }
        }
        heading
    }

    /// Track starts of row `index`'s presentation, to tell a restarted run
    /// from one left alone.
    fn started(&self, index: usize) -> Option<Duration> {
        let world = self.context.world();
        let target = crate::MotionTargetId::new(self.rows[index].get())?;
        world
            .presentation_store()
            .overlays()
            .find(|overlay| overlay.track.target == target)
            .map(|overlay| overlay.track.timing.start)
    }

    fn row_overlays(&self) -> usize {
        let world = self.context.world();
        world
            .presentation_store()
            .overlays()
            .filter(|overlay| {
                self.rows
                    .iter()
                    .any(|row| crate::MotionTargetId::new(row.get()) == Some(overlay.track.target))
            })
            .count()
    }

    fn z_index(&self, id: StableNodeId) -> i32 {
        self.context.world().extract_nodes(&[id])[0].z_index
    }

    fn advance(&mut self, by: Duration) {
        let now = self.context.animation_now() + by;
        let _ = self.context.advance_animations(now);
    }

    fn take_events(&self) -> Vec<ReorderListEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

/// The height and gap a carried row frees when it leaves its place.
const BLOCK: f32 = ROW + GAP;

#[test]
fn a_live_drag_moves_rows_only_on_the_presentation_layer() {
    let mut rows = LiveRows::new(false);
    let styles: Vec<_> = rows
        .rows
        .iter()
        .chain(&rows.inner)
        .map(|id| rows.context.world().node_style(*id).cloned())
        .collect();
    let (x, y) = rows.press(0);
    // Under the threshold nothing moves.
    rows.move_to(x, y + 2.0);
    assert_eq!(rows.row_overlays(), 0);

    // Past the middle of row `c`: `a` lands before `d`, so `b` and `c` move up
    // into its place.
    let to = rows.boxes[2].y + 30.0;
    rows.move_to(x + 6.0, to);
    assert_eq!(rows.presented(0).map(|t| (t.e, t.f)), Some((6.0, to - y)));
    assert_eq!(rows.heading(1), Some([0.0, -BLOCK]));
    assert_eq!(rows.heading(2), Some([0.0, -BLOCK]));
    assert_eq!(rows.heading(3), None, "`d` stays where it is");
    rows.advance(Duration::from_secs(1));
    assert_eq!(rows.presented(1).map(|t| t.f), Some(-BLOCK));
    assert_eq!(rows.presented(2).map(|t| t.f), Some(-BLOCK));

    // Logical state is untouched: styles and layout boxes as before.
    let now: Vec<_> = rows
        .rows
        .iter()
        .chain(&rows.inner)
        .map(|id| rows.context.world().node_style(*id).cloned())
        .collect();
    assert_eq!(now, styles);
    for (row, laid_out) in rows.rows.iter().zip(&rows.boxes) {
        assert_eq!(rows.context.world().layout_box(*row), Some(*laid_out));
    }

    // The list draws no outline or line; the gap says where the row lands.
    let layout = rows
        .context
        .world()
        .reorder_list_layout(rows.list.stable_id())
        .expect("reorder list layout");
    assert_eq!((layout.insert, layout.dragged), (None, None));

    // The carried row and what it holds stack over the other rows, under an
    // open menu.
    assert_eq!(rows.z_index(rows.rows[0]), REORDER_PREVIEW_Z_INDEX);
    assert_eq!(rows.z_index(rows.inner[0]), REORDER_PREVIEW_Z_INDEX);
    assert_eq!(rows.z_index(rows.rows[1]), 0);
    const { assert!(REORDER_PREVIEW_Z_INDEX < crate::MENU_OVERLAY_Z_INDEX) };
}

#[test]
fn a_new_landing_restarts_only_the_rows_it_moves_and_extracts_none() {
    let mut rows = LiveRows::new(false);
    let (x, y) = rows.press(0);
    rows.move_to(x, rows.boxes[2].y + 30.0);
    rows.advance(Duration::from_millis(16));
    let _ = rows.context.take_system_work();
    let (b, c) = (rows.started(1), rows.started(2));

    // A move that keeps the landing only moves the carried row.
    rows.move_to(x, rows.boxes[2].y + 32.0);
    let work = rows.context.take_system_work();
    assert!(
        work.render_extraction.is_empty(),
        "a pointer move re-extracted {:?}",
        work.render_extraction
    );
    assert_eq!((rows.started(1), rows.started(2)), (b, c));

    // Past the middle of `d`: it moves up too, and only it starts a run.
    rows.move_to(x, rows.boxes[3].y + 30.0);
    let work = rows.context.take_system_work();
    assert!(
        work.render_extraction.is_empty(),
        "a new landing re-extracted {:?}",
        work.render_extraction
    );
    assert_eq!((rows.started(1), rows.started(2)), (b, c));
    assert_eq!(rows.heading(3), Some([0.0, -BLOCK]));
    let _ = y;

    // Back above `b`'s middle: `a` would stay put, so every row heads home.
    rows.move_to(x, rows.boxes[0].y + 10.0);
    for index in 1..4 {
        assert_eq!(rows.heading(index), Some([0.0, 0.0]), "row {index}");
    }
    rows.advance(Duration::from_secs(1));
    for index in 1..4 {
        assert_eq!(rows.presented(index).map(|t| t.f), Some(0.0), "row {index}");
    }
}

#[test]
fn moving_up_shifts_the_rows_between_down() {
    let mut rows = LiveRows::new(false);
    let (x, _) = rows.press(3);
    // Above the middle of `b`: `d` lands before `b`.
    rows.move_to(x, rows.boxes[1].y + 10.0);
    assert_eq!(rows.heading(0), None);
    assert_eq!(rows.heading(1), Some([0.0, BLOCK]));
    assert_eq!(rows.heading(2), Some([0.0, BLOCK]));
}

#[test]
fn cancel_and_release_clear_every_presentation_and_restack() {
    let mut rows = LiveRows::new(false);
    let (x, _) = rows.press(0);
    rows.move_to(x, rows.boxes[2].y + 30.0);
    assert!(rows.row_overlays() > 0);
    let _ = rows.context.take_system_work();

    assert!(rows.context.cancel_reorder_list_drag(document()).unwrap());
    assert_eq!(rows.take_events(), vec![ReorderListEvent::Cancelled]);
    assert_eq!(rows.row_overlays(), 0, "cancel left a presentation");
    assert_eq!(rows.z_index(rows.rows[0]), 0);
    assert_eq!(rows.z_index(rows.inner[0]), 0);
    // The carried row's stacking went back: it, not the others, re-extracts.
    let work = rows.context.take_system_work();
    assert!(work.render_extraction.contains(&rows.rows[0]));
    assert!(!work.render_extraction.contains(&rows.rows[1]));

    let (x, _) = rows.press(0);
    let to = rows.boxes[2].y + 30.0;
    rows.move_to(x, to);
    assert!(rows.row_overlays() > 0);
    rows.context
        .end_reorder_list_pointer(document(), 1, x, to, false)
        .unwrap();
    assert_eq!(
        rows.take_events(),
        vec![ReorderListEvent::Reorder {
            source: "a".into(),
            before: Some("d".into()),
        }]
    );
    assert_eq!(rows.row_overlays(), 0, "release left a presentation");
    assert_eq!(rows.z_index(rows.rows[0]), 0);
}

#[test]
fn reduced_motion_makes_room_at_once_and_still_follows_the_pointer() {
    let mut rows = LiveRows::new(false);
    rows.context.set_reduced_motion(true);
    let (x, y) = rows.press(0);
    let to = rows.boxes[2].y + 30.0;
    rows.move_to(x, to);
    assert_eq!(rows.presented(0).map(|t| t.f), Some(to - y));
    assert_eq!(rows.presented(1).map(|t| t.f), Some(-BLOCK));
    assert_eq!(rows.presented(2).map(|t| t.f), Some(-BLOCK));
}

#[test]
fn a_tree_drop_keeps_its_marks_and_moves_no_row() {
    let mut rows = LiveRows::new(true);
    let (x, _) = rows.press(0);
    rows.move_to(x, rows.boxes[2].y + 38.0);
    assert_eq!(rows.row_overlays(), 0);
    assert_eq!(rows.z_index(rows.rows[0]), 0);
    let layout = rows
        .context
        .world()
        .reorder_list_layout(rows.list.stable_id())
        .expect("reorder list layout");
    assert_eq!(layout.dragged.map(|(row, _)| row), Some(rows.boxes[0]));
}

#[test]
fn live_gap_tree_shifts_nested_slot_folded_and_offset_paint() {
    let mut context = AppContext::new();
    let wrapper = context
        .create_component(
            document(),
            crate::Stack::column(0.0).with_layout(|layout| {
                layout.width = Some(LengthSpec::Fill);
                layout.padding_left = Some(LengthSpec::Px(24.0));
                layout.padding_top = Some(LengthSpec::Px(30.0));
            }),
        )
        .unwrap();
    let list = context
        .create_detached_component(
            document(),
            ReorderList::new([
                ReorderItem::new("a", "a"),
                ReorderItem::new("p", "p").accepts_inside(ReorderAccepts::All),
                ReorderItem::new("c1", "c1").depth(1),
                ReorderItem::new("f", "f")
                    .accepts_inside(ReorderAccepts::All)
                    .folded(true),
                ReorderItem::new("d", "d"),
            ])
            .live_rows(true)
            .tree_drop(true)
            .drag_preview(ReorderDragPreview::LiveGap)
            .spacing(GAP),
        )
        .unwrap();
    context.append_child(wrapper, list).unwrap();
    let mut row_ids = Vec::new();
    for _ in 0..5 {
        let row = context
            .create_detached_component(
                document(),
                crate::Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Fill);
                    layout.height = Some(LengthSpec::Px(ROW));
                }),
            )
            .unwrap();
        context.append_child(list, row).unwrap();
        row_ids.push(row.stable_id());
    }
    context
        .layout_document(document(), LayoutViewport::new(260.0, 400.0))
        .unwrap();

    let list_box = context.world().layout_box(list.stable_id()).unwrap();
    assert_eq!(list_box.x, 24.0);
    assert_eq!(list_box.y, 30.0);
    let boxes: Vec<_> = row_ids
        .iter()
        .map(|id| context.world().layout_box(*id).unwrap())
        .collect();

    // Drag `a` (index 0) into middle 50% of open parent `p` (index 1).
    let start_x = boxes[0].x + 20.0;
    let start_y = boxes[0].y + 20.0;
    assert!(
        context
            .begin_reorder_list_pointer(document(), 1, list.stable_id(), start_x, start_y)
            .unwrap()
    );
    context
        .update_reorder_list_pointer(document(), 1, boxes[1].x + 20.0, boxes[1].y + ROW * 0.5)
        .unwrap();

    fn heading_of(context: &AppContext, row_id: StableNodeId) -> [f32; 2] {
        let world = context.world();
        let target = crate::MotionTargetId::new(row_id.get()).unwrap();
        for overlay in world.presentation_store().overlays() {
            if overlay.track.target == target
                && let crate::MotionTo::Value(MotionValue::Transform(to)) = overlay.track.to
            {
                return [to.e, to.f];
            }
        }
        [0.0, 0.0]
    }

    // `p` slides up into `a`'s spot; `c1` and below stay put; indented nested slot opens.
    assert_eq!(heading_of(&context, row_ids[1]), [0.0, -BLOCK]);
    assert_eq!(heading_of(&context, row_ids[2]), [0.0, 0.0]);
    let layout = context
        .world()
        .reorder_list_layout(list.stable_id())
        .expect("live_gap layout");
    let expected_p = LayoutBox {
        y: boxes[1].y - BLOCK,
        ..boxes[1]
    };
    assert_eq!(layout.inside.map(|(r, _)| r), Some(expected_p));
    let (slot, _, nested) = layout.slot.expect("nested dashed slot");
    assert!(nested);
    assert!((slot.x - (list_box.x + DEFAULT_INDENT)).abs() < 0.01);
    assert!((slot.y - (boxes[2].y - BLOCK)).abs() < 0.01);

    // Extracted Vector op uses list-local coordinates (not double-offset by wrapper padding).
    let extracted = context.world().extract_nodes(&[list.stable_id()]);
    let widget_paint = extracted[0].widget_paint.as_ref().expect("widget paint");
    let vec_bounds = widget_paint
        .ops
        .iter()
        .find_map(|op| match &op.op {
            crate::WidgetPaintOp::Vector { bounds, .. } => Some(*bounds),
            _ => None,
        })
        .expect("vector slot op");
    assert!((vec_bounds.x - 0.0).abs() < 0.01);
    assert!(
        (vec_bounds.y - ROW).abs() < 0.01,
        "local paint origin must match shifted `p` bottom in list-local space ({ROW}), got {vec_bounds:?}"
    );

    // Hovering folded parent `f` (index 3) highlights container without shifting or opening slot.
    context
        .update_reorder_list_pointer(document(), 1, boxes[3].x + 20.0, boxes[3].y + ROW * 0.5)
        .unwrap();
    let folded_layout = context
        .world()
        .reorder_list_layout(list.stable_id())
        .expect("folded layout");
    assert_eq!(folded_layout.slot, None);
    assert_eq!(folded_layout.inside.map(|(r, _)| r), Some(boxes[3]));
    assert_eq!(heading_of(&context, row_ids[1]), [0.0, 0.0]);
    assert_eq!(heading_of(&context, row_ids[4]), [0.0, 0.0]);
}

#[test]
fn live_gap_grid_2d_shifts_and_external_drag() {
    let mut context = AppContext::new();
    let list = context
        .create_component(
            document(),
            ReorderList::new(["t0", "t1", "t2", "t3", "t4"].map(|v| ReorderItem::new(v, v)))
                .live_rows(true)
                .layout_mode(ReorderLayout::Grid)
                .drag_preview(ReorderDragPreview::LiveGap)
                .spacing(8.0)
                .column_gap(8.0),
        )
        .unwrap();
    let mut tile_ids = Vec::new();
    for _ in 0..5 {
        let tile = context
            .create_detached_component(
                document(),
                crate::Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(80.0));
                    layout.height = Some(LengthSpec::Px(60.0));
                }),
            )
            .unwrap();
        context.append_child(list, tile).unwrap();
        tile_ids.push(tile.stable_id());
    }
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = Arc::clone(&events);
    context
        .on(list, move |_, event: &ReorderListEvent, _| {
            observed.lock().unwrap().push(event.clone());
        })
        .unwrap();
    context
        .layout_document(document(), LayoutViewport::new(280.0, 400.0))
        .unwrap();

    let boxes: Vec<_> = tile_ids
        .iter()
        .map(|id| context.world().layout_box(*id).unwrap())
        .collect();
    assert_eq!(boxes[0].y, boxes[2].y);
    assert!(boxes[3].y > boxes[0].y);

    // Drag `t0` onto `t3`'s right half (`before = Some("t4")`, target slot index 3).
    assert!(
        context
            .begin_reorder_list_pointer(
                document(),
                1,
                list.stable_id(),
                boxes[0].x + 20.0,
                boxes[0].y + 20.0
            )
            .unwrap()
    );
    let drop_x = boxes[3].x + 60.0;
    let drop_y = boxes[3].y + 30.0;
    context
        .update_reorder_list_pointer(document(), 1, drop_x, drop_y)
        .unwrap();

    let world = context.world();
    let target3 = crate::MotionTargetId::new(tile_ids[3].get()).unwrap();
    let heading3 = world
        .presentation_store()
        .overlays()
        .find_map(|overlay| {
            (overlay.track.target == target3)
                .then(|| match overlay.track.to {
                    crate::MotionTo::Value(MotionValue::Transform(to)) => Some([to.e, to.f]),
                    _ => None,
                })
                .flatten()
        })
        .expect("tile 3 wrapped up-right");
    assert!((heading3[0] - (boxes[2].x - boxes[3].x)).abs() < 0.01);
    assert!((heading3[1] - (boxes[2].y - boxes[3].y)).abs() < 0.01);

    let layout = world.reorder_list_layout(list.stable_id()).unwrap();
    assert_eq!(layout.slot.map(|(r, _, _)| r), Some(boxes[3]));

    context
        .end_reorder_list_pointer(document(), 1, drop_x, drop_y, false)
        .unwrap();
    let reorders: Vec<_> = events
        .lock()
        .unwrap()
        .drain(..)
        .filter(|e| matches!(e, ReorderListEvent::Reorder { .. }))
        .collect();
    assert_eq!(
        reorders,
        vec![ReorderListEvent::Reorder {
            source: "t0".into(),
            before: Some("t4".into()),
        }]
    );

    // External drag onto left half of `t1` pushes remaining tiles in 2D wrap order.
    let ext_x = boxes[1].x + 15.0;
    let ext_y = boxes[1].y + 30.0;
    assert!(
        context
            .set_reorder_external_drag(
                list.stable_id(),
                Some(ReorderExternalDrag {
                    value: "ext".into(),
                    kind: None,
                    x: ext_x,
                    y: ext_y,
                    size: Some([80.0, 60.0]),
                })
            )
            .unwrap()
    );
    let ext_layout = context
        .world()
        .reorder_list_layout(list.stable_id())
        .unwrap();
    assert_eq!(ext_layout.slot.map(|(r, _, _)| r), Some(boxes[1]));
    let ext_event = context
        .finish_reorder_external_drag(list.stable_id())
        .unwrap();
    assert_eq!(
        ext_event,
        Some(ReorderListEvent::Reorder {
            source: "ext".into(),
            before: Some("t1".into()),
        })
    );

    // Empty grid accepts external drag at root and opens a slot.
    let mut empty_ctx = AppContext::new();
    let empty_grid = empty_ctx
        .create_component(
            document(),
            ReorderList::new(std::iter::empty::<ReorderItem>())
                .live_rows(true)
                .layout_mode(ReorderLayout::Grid)
                .drag_preview(ReorderDragPreview::LiveGap)
                .style({
                    let mut style = NodeStyle::default();
                    style.layout_mut().min_height = Some(LengthSpec::Px(90.0));
                    style
                }),
        )
        .unwrap();
    empty_ctx
        .layout_document(document(), LayoutViewport::new(280.0, 200.0))
        .unwrap();
    assert!(
        empty_ctx
            .set_reorder_external_drag(
                empty_grid.stable_id(),
                Some(ReorderExternalDrag {
                    value: "ext".into(),
                    kind: None,
                    x: 40.0,
                    y: 40.0,
                    size: Some([80.0, 60.0]),
                })
            )
            .unwrap()
    );
    let empty_layout = empty_ctx
        .world()
        .reorder_list_layout(empty_grid.stable_id())
        .unwrap();
    assert!(empty_layout.slot.is_some());
    assert_eq!(
        empty_ctx
            .finish_reorder_external_drag(empty_grid.stable_id())
            .unwrap(),
        Some(ReorderListEvent::Reorder {
            source: "ext".into(),
            before: None,
        })
    );
}

#[test]
fn live_gap_transfer_group_drop_zone_and_outside_drag_event() {
    let mut context = AppContext::new();
    let root = context
        .create_component(document(), crate::Stack::column(12.0))
        .unwrap();

    let card_a = context
        .create_detached_component(
            document(),
            crate::Stack::column(0.0).with_layout(|layout| {
                layout.width = Some(LengthSpec::Fill);
                layout.padding_bottom = Some(LengthSpec::Px(12.0));
            }),
        )
        .unwrap();
    context.append_child(root, card_a).unwrap();

    let list_a = context
        .create_detached_component(
            document(),
            ReorderList::new(["a0", "a1"].map(|v| ReorderItem::new(v, v)))
                .live_rows(true)
                .drag_preview(ReorderDragPreview::LiveGap)
                .transfer_group("cards")
                .drop_zone(Some(card_a.stable_id()))
                .spacing(GAP),
        )
        .unwrap();
    context.append_child(card_a, list_a).unwrap();
    let mut rows_a = Vec::new();
    for _ in 0..2 {
        let row = context
            .create_detached_component(
                document(),
                crate::Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Fill);
                    layout.height = Some(LengthSpec::Px(ROW));
                }),
            )
            .unwrap();
        context.append_child(list_a, row).unwrap();
        rows_a.push(row.stable_id());
    }

    let card_b = context
        .create_detached_component(
            document(),
            crate::Stack::column(0.0).with_layout(|layout| {
                layout.width = Some(LengthSpec::Fill);
                layout.min_height = Some(LengthSpec::Px(80.0));
                layout.padding_top = Some(LengthSpec::Px(24.0));
                layout.pointer_events = Some(nana_ui_core::PointerEventsSpec::Auto);
            }),
        )
        .unwrap();
    context.append_child(root, card_b).unwrap();

    let list_b = context
        .create_detached_component(
            document(),
            ReorderList::new(std::iter::empty::<ReorderItem>())
                .live_rows(true)
                .drag_preview(ReorderDragPreview::LiveGap)
                .transfer_group("cards")
                .drop_zone(Some(card_b.stable_id()))
                .spacing(GAP),
        )
        .unwrap();
    context.append_child(card_b, list_b).unwrap();

    let events_a = Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_b = Arc::new(std::sync::Mutex::new(Vec::new()));
    let obs_a = Arc::clone(&events_a);
    let obs_b = Arc::clone(&events_b);
    context
        .on(list_a, move |_, e: &ReorderListEvent, _| {
            obs_a.lock().unwrap().push(e.clone());
        })
        .unwrap();
    context
        .on(list_b, move |_, e: &ReorderListEvent, _| {
            obs_b.lock().unwrap().push(e.clone());
        })
        .unwrap();

    context
        .layout_document(document(), LayoutViewport::new(300.0, 400.0))
        .unwrap();
    let box_a0 = context.world().layout_box(rows_a[0]).unwrap();
    let box_card_b = context.world().layout_box(card_b.stable_id()).unwrap();

    // Drag `a0` into card_b's drop_zone header area (list_b itself is empty / 0px high).
    assert!(
        context
            .begin_reorder_list_pointer(
                document(),
                1,
                list_a.stable_id(),
                box_a0.x + 20.0,
                box_a0.y + 20.0
            )
            .unwrap()
    );
    context
        .update_reorder_list_pointer(document(), 1, box_card_b.x + 30.0, box_card_b.y + 12.0)
        .unwrap();

    // Re-layout so list_b's incoming padding_bottom takes effect, then inspect slot & collapse.
    context
        .layout_document(document(), LayoutViewport::new(300.0, 400.0))
        .unwrap();
    let layout_b = context
        .world()
        .reorder_list_layout(list_b.stable_id())
        .unwrap();
    assert!(layout_b.slot.is_some());
    let layout_a = context
        .world()
        .reorder_list_layout(list_a.stable_id())
        .unwrap();
    assert_eq!(layout_a.slot, None);
    let target_a1 = crate::MotionTargetId::new(rows_a[1].get()).unwrap();
    let shift_a1 = context
        .world()
        .presentation_store()
        .overlays()
        .find_map(|o| {
            (o.track.target == target_a1)
                .then(|| match o.track.to {
                    crate::MotionTo::Value(MotionValue::Transform(to)) => Some(to.f),
                    _ => None,
                })
                .flatten()
        });
    assert_eq!(shift_a1, Some(-BLOCK));

    // Releasing over card_b transfers `a0` into empty `list_b`.
    context
        .end_reorder_list_pointer(
            document(),
            1,
            box_card_b.x + 30.0,
            box_card_b.y + 12.0,
            false,
        )
        .unwrap();
    assert_eq!(
        *events_b.lock().unwrap(),
        vec![ReorderListEvent::Reorder {
            source: "a0".into(),
            before: None,
        }]
    );
    assert!(events_a.lock().unwrap().iter().any(|e| matches!(
        e,
        ReorderListEvent::Drag {
            source,
            phase: PointerDragPhase::Ended,
            inside_group: true,
            ..
        } if &**source == "a0"
    )));
    events_a.lock().unwrap().clear();
    events_b.lock().unwrap().clear();

    // Dragging outside both drop_zones emits Drag { inside_group: false } and NO Reorder.
    assert!(
        context
            .begin_reorder_list_pointer(
                document(),
                1,
                list_a.stable_id(),
                box_a0.x + 20.0,
                box_a0.y + 20.0
            )
            .unwrap()
    );
    context
        .update_reorder_list_pointer(document(), 1, box_a0.x + 20.0, 350.0)
        .unwrap();
    context
        .end_reorder_list_pointer(document(), 1, box_a0.x + 20.0, 350.0, false)
        .unwrap();
    let drained_a: Vec<_> = events_a.lock().unwrap().drain(..).collect();
    assert!(
        !drained_a
            .iter()
            .any(|e| matches!(e, ReorderListEvent::Reorder { .. }))
    );
    assert!(drained_a.iter().any(|e| matches!(
        e,
        ReorderListEvent::Drag {
            source,
            phase: PointerDragPhase::Ended,
            inside_group: false,
            ..
        } if &**source == "a0"
    )));
}
