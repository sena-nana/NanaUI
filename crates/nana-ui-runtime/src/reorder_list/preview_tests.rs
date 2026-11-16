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
