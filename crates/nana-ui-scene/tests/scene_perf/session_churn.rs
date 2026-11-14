//! What a session holds follows what is on screen, not how long it has run.
//!
//! Node ids are handed out in order and never reused, and several things
//! grew with them: every node ever removed left a layout cause queued that
//! each frame looked at again (70,000 removals made every frame 3 ms
//! slower); the tables indexed by id kept a slot per id ever issued (about
//! half a kilobyte each, 33 MB by the time the ids passed 65,000); and the
//! set of nodes whose layout result is withheld and the memo of which boxes
//! hold a line box each kept an entry per removed node.

use crate::{LIVE, serial_lock};
use std::sync::atomic::Ordering;

use nana_ui_runtime::{Button, DocumentId, Entity, LayoutViewport, MeasureTextShaper, Stack, Text};
use nana_ui_scene::RuntimeDocument;

fn frame(runtime: &mut RuntimeDocument) {
    let viewport = LayoutViewport::new(400.0, 600.0);
    for _ in 0..2 {
        runtime.flush(viewport, &mut MeasureTextShaper).unwrap();
    }
}

/// A row of a label and a button, last in `column`.
fn add_row(runtime: &mut RuntimeDocument, column: Entity<Stack>, index: usize) -> Entity<Stack> {
    let document = DocumentId::new(1).unwrap();
    let cx = runtime.context_mut();
    let row = cx
        .create_detached_component(document, Stack::row(4.0))
        .unwrap();
    cx.append_child(column, row).unwrap();
    let label = cx
        .create_detached_component(document, Text::new(format!("row {index}")))
        .unwrap();
    cx.append_child(row, label).unwrap();
    let button = cx
        .create_detached_component(document, Button::new("Open"))
        .unwrap();
    cx.append_child(row, button).unwrap();
    row
}

/// Ten rows shown for a frame and removed again.
fn come_and_go(runtime: &mut RuntimeDocument, column: Entity<Stack>) {
    let rows: Vec<_> = (0..10)
        .map(|index| add_row(runtime, column, index))
        .collect();
    frame(runtime);
    for row in rows {
        runtime.context_mut().remove_view(row).unwrap();
    }
    frame(runtime);
}

#[test]
fn rows_coming_and_going_leave_nothing_behind() {
    let _serial = serial_lock();
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let column = runtime
        .context_mut()
        .create_component(document, Stack::column(2.0))
        .unwrap();
    for index in 0..40 {
        add_row(&mut runtime, column, index);
    }
    frame(&mut runtime);

    // Tables reach the capacity ten more rows need.
    for _ in 0..100 {
        come_and_go(&mut runtime, column);
    }
    let settled = LIVE.load(Ordering::Relaxed);
    let nodes = runtime.context().world().len();

    // Eighteen thousand more nodes created and removed.
    const CYCLES: isize = 600;
    for _ in 0..CYCLES {
        come_and_go(&mut runtime, column);
    }
    assert_eq!(runtime.context().world().len(), nodes);
    let grown = LIVE.load(Ordering::Relaxed) - settled;
    // What is left to grow is the directory of pages of each table indexed
    // by id: eight bytes per sixty-four ids between the oldest node alive
    // and the newest, under two bytes a node over all of them. A table
    // that keeps anything per removed node keeps eight bytes or more.
    let created = CYCLES * 30;
    assert!(
        grown < created * 4,
        "{grown} bytes more are held after {created} nodes came and went: {} a node",
        grown / created
    );
}

#[test]
fn grouped_escaped_and_queried_rows_leave_nothing_behind() {
    let _serial = serial_lock();
    use nana_ui_core::motion::{AnimatableProperty, Easing, MotionTo, MotionValue};
    use nana_ui_core::{LayoutStyle, LengthSpec, OverflowSpec, PositionSpec};
    use nana_ui_runtime::{AnimationId, AnimationSpec, MutationQueue};
    use std::sync::Arc;
    use std::time::Duration;

    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let column = runtime
        .context_mut()
        .create_component(
            document,
            Stack::column(2.0).with_layout(|layout| {
                layout.opacity = Some(0.9);
            }),
        )
        .unwrap();
    let mut now = Duration::ZERO;
    let mut next_anim = 1u64;
    {
        let mut queue = MutationQueue::new();
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(next_anim).unwrap(),
                column.stable_id(),
                now,
                Duration::from_secs(3600),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Opacity)
            .with_range(
                MotionValue::Scalar(0.85),
                MotionTo::Value(MotionValue::Scalar(0.95)),
            ),
        );
        next_anim += 1;
        runtime
            .context_mut()
            .compat_world_mut()
            .commit(queue)
            .unwrap();
    }
    runtime
        .context_mut()
        .compat_world_mut()
        .advance_animations(now);
    frame(&mut runtime);
    now += Duration::from_millis(20);
    runtime
        .context_mut()
        .compat_world_mut()
        .advance_animations(now);
    frame(&mut runtime);
    assert!(
        runtime
            .scene()
            .compositor_layer(column.stable_id())
            .is_some()
    );

    let mut cycle = |runtime: &mut RuntimeDocument| {
        let mut rows = Vec::new();
        let mut anim_queue = MutationQueue::new();
        for index in 0..8 {
            let cx = runtime.context_mut();
            let clip_box = cx
                .create_detached_component(
                    document,
                    Stack::row(2.0).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(120.0));
                        layout.height = Some(LengthSpec::Px(24.0));
                        layout.overflow_x = OverflowSpec::Hidden;
                        layout.overflow_y = OverflowSpec::Hidden;
                        layout.opacity = Some(0.85);
                    }),
                )
                .unwrap();
            cx.append_child(column, clip_box).unwrap();
            anim_queue.start_animation(
                AnimationSpec::new(
                    AnimationId::new(next_anim).unwrap(),
                    clip_box.stable_id(),
                    now,
                    Duration::from_secs(3600),
                    Duration::from_millis(16),
                    Easing::Linear,
                )
                .with_property(AnimatableProperty::Opacity)
                .with_range(
                    MotionValue::Scalar(0.7),
                    MotionTo::Value(MotionValue::Scalar(0.9)),
                ),
            );
            next_anim += 1;

            let label = cx
                .create_detached_component(document, Text::new(format!("line {index}")))
                .unwrap();
            cx.append_child(clip_box, label).unwrap();

            let mut badge_layout = LayoutStyle::default();
            badge_layout.position = PositionSpec::Absolute;
            badge_layout.offset_left = Some(LengthSpec::Px(8.0));
            badge_layout.offset_top = Some(LengthSpec::Px(2.0));
            let badge = cx
                .create_detached_component(
                    document,
                    Button::new(format!("badge {index}")).layout(Arc::new(badge_layout)),
                )
                .unwrap();
            cx.append_child(clip_box, badge).unwrap();
            rows.push((clip_box, badge));
        }
        runtime
            .context_mut()
            .compat_world_mut()
            .commit(anim_queue)
            .unwrap();
        runtime
            .context_mut()
            .compat_world_mut()
            .advance_animations(now);
        frame(runtime);
        now += Duration::from_millis(20);
        runtime
            .context_mut()
            .compat_world_mut()
            .advance_animations(now);
        frame(runtime);
        for primitive in runtime.scene().primitives() {
            let _ = runtime.scene().draw_primitive(primitive.id);
            let _ = runtime.scene().opacity_groups(primitive.node);
        }
        let _ = runtime.context().world().hit_test(document, 12.0, 6.0);
        for &(clip_box, badge) in &rows {
            assert!(
                runtime
                    .scene()
                    .compositor_layer(clip_box.stable_id())
                    .is_some()
            );
            assert!(
                runtime
                    .context()
                    .world()
                    .layout_result(clip_box.stable_id())
                    .is_some()
            );
            let _ = runtime.context().world().layout_result(badge.stable_id());
        }
        for (clip_box, _) in rows {
            runtime.context_mut().remove_view(clip_box).unwrap();
        }
        frame(runtime);
        for primitive in runtime.scene().primitives() {
            let _ = runtime.scene().draw_primitive(primitive.id);
            let _ = runtime.scene().opacity_groups(primitive.node);
        }
        let _ = runtime.context().world().hit_test(document, 12.0, 6.0);
    };

    for _ in 0..40 {
        cycle(&mut runtime);
    }
    let settled = LIVE.load(Ordering::Relaxed);
    let nodes = runtime.context().world().len();

    const CYCLES: isize = 200;
    for _ in 0..CYCLES {
        cycle(&mut runtime);
    }
    assert_eq!(runtime.context().world().len(), nodes);
    let grown = LIVE.load(Ordering::Relaxed) - settled;
    let created = CYCLES * 32;
    assert!(
        grown < created * 6,
        "{grown} bytes more are held after {created} grouped/escaped nodes came and went: {} a node",
        grown / created
    );
}
