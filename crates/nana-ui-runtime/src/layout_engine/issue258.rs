//! Issue #258 overlay placement and context-plan memory.
//!
//! Owned by the overlay slice so the inline formatting work can keep editing
//! `tests.rs` and `inline.rs`.
#![cfg(test)]

use std::sync::Arc;

use nana_ui_core::{AlignSpec, FlexDirection, LayoutStyle, LengthSpec, PositionSpec};

use crate::{DocumentId, MutationQueue, NodeKind, NodeStyle, UiWorld};

use super::*;

fn nid(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

fn px(value: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(value)),
        height: Some(LengthSpec::Px(20.0)),
        ..LayoutStyle::default()
    }
}

fn write_changed(world: &mut UiWorld, emitted: &[(StableNodeId, LayoutBox)]) {
    let mut queue = MutationQueue::new();
    let mut any = false;
    for (id, box_) in emitted {
        if world.layout_box(*id) != Some(*box_) {
            queue.write_layout(*id, *box_);
            any = true;
        }
    }
    if any {
        world.commit(queue).unwrap();
    }
}

fn full_boxes(
    world: &UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
) -> Vec<(StableNodeId, LayoutBox)> {
    RuntimeLayoutEngine
        .layout_document(world, document, viewport)
        .unwrap()
}

struct Step {
    emitted: Vec<StableNodeId>,
    children_measured: usize,
    full_document_fallbacks: usize,
}

fn scoped(
    world: &mut UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
    retained: &mut RetainedLayoutCache,
) -> Step {
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(
            world,
            document,
            viewport,
            &work.layout_frontier_seeds,
            retained,
            false,
        )
        .unwrap();
    let cache = &retained.documents[&document];
    let step = Step {
        emitted: emitted.iter().map(|(id, _)| *id).collect(),
        children_measured: cache.execution_stats.children_measured,
        full_document_fallbacks: cache.frontier_stats.full_document_fallbacks,
    };
    write_changed(world, &emitted);
    let _ = world.take_system_work();
    let expected = full_boxes(world, document, viewport);
    let cached = &retained.documents[&document].boxes;
    for (id, box_) in expected {
        assert_eq!(
            cached.get(&id).copied().unwrap_or_default(),
            box_,
            "scoped layout diverged at {id:?}"
        );
    }
    step
}

fn prime(
    world: &mut UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
    retained: &mut RetainedLayoutCache,
) {
    let _ = world.take_system_work();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(world, document, viewport, &[], retained, true)
        .unwrap();
    write_changed(world, &emitted);
    let _ = world.take_system_work();
}

fn set_layout(world: &mut UiWorld, id: StableNodeId, layout: LayoutStyle) {
    let mut queue = MutationQueue::new();
    queue.set_style(
        id,
        NodeStyle {
            layout: Arc::new(layout),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
}

/// `in_flow` fixed rows, one absolute child, one fixed child, and an unrelated
/// sibling outside the container.
fn positioned_world(in_flow: usize, absolute: LayoutStyle) -> (UiWorld, DocumentId) {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(nid(1), document, NodeKind::Document);
    queue.create(nid(2), document, NodeKind::Element { tag: "div".into() });
    queue.create(nid(3), document, NodeKind::Element { tag: "div".into() });
    queue.insert(nid(1), nid(2), None);
    queue.insert(nid(1), nid(3), None);
    queue.set_style(
        nid(2),
        NodeStyle {
            layout: Arc::new(px(40.0)),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        nid(3),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(400.0)),
                height: Some(LengthSpec::Px(300.0)),
                direction: Some(FlexDirection::Column),
                align_items: AlignSpec::Start,
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for index in 0..in_flow {
        let id = nid(10 + index as u64);
        queue.create(id, document, NodeKind::Element { tag: "div".into() });
        queue.insert(nid(3), id, None);
        queue.set_style(
            id,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    queue.create(nid(90), document, NodeKind::Element { tag: "div".into() });
    queue.create(nid(91), document, NodeKind::Element { tag: "div".into() });
    queue.insert(nid(3), nid(90), None);
    queue.insert(nid(3), nid(91), None);
    queue.set_style(
        nid(90),
        NodeStyle {
            layout: Arc::new(absolute),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        nid(91),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                position: PositionSpec::Fixed,
                offset_left: Some(LengthSpec::Px(4.0)),
                offset_top: Some(LengthSpec::Px(4.0)),
                width: Some(LengthSpec::Px(16.0)),
                height: Some(LengthSpec::Px(16.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    (world, document)
}

fn absolute_px(width: f32) -> LayoutStyle {
    LayoutStyle {
        position: PositionSpec::Absolute,
        offset_left: Some(LengthSpec::Px(8.0)),
        offset_top: Some(LengthSpec::Px(8.0)),
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(24.0)),
        ..LayoutStyle::default()
    }
}

fn assert_flow_not_placed(step: &Step, in_flow: usize) {
    for index in 0..in_flow {
        assert!(
            !step.emitted.contains(&nid(10 + index as u64)),
            "in-flow sibling {} was placed: {:?}",
            index,
            step.emitted
        );
    }
    assert!(
        !step.emitted.contains(&nid(2)),
        "unrelated branch was placed: {:?}",
        step.emitted
    );
    assert_eq!(step.full_document_fallbacks, 0);
}

/// A positioned child's own size change places that child and leaves in-flow
/// siblings, and an unrelated branch, unplaced.
#[test]
fn positioned_content_change_does_not_place_in_flow_siblings() {
    let viewport = LayoutViewport::new(800.0, 600.0);
    let mut measured = Vec::new();
    for in_flow in [4usize, 16] {
        let (mut world, document) = positioned_world(in_flow, absolute_px(40.0));
        let mut retained = RetainedLayoutCache::default();
        prime(&mut world, document, viewport, &mut retained);

        set_layout(&mut world, nid(90), absolute_px(55.0));
        let step = scoped(&mut world, document, viewport, &mut retained);
        assert!(
            step.emitted.contains(&nid(90)),
            "absolute child was not placed: {:?}",
            step.emitted
        );
        assert_flow_not_placed(&step, in_flow);
        assert!(
            step.children_measured < in_flow,
            "{in_flow} in-flow siblings measured {}",
            step.children_measured
        );
        assert_eq!(world.layout_box(nid(90)).unwrap().width, 55.0);
        measured.push(step.children_measured);

        let mut painted = absolute_px(55.0);
        painted.opacity = Some(0.4);
        set_layout(&mut world, nid(90), painted);
        let work = world.take_system_work();
        assert!(
            work.layout_frontier_seeds.is_empty(),
            "paint-only on an absolute child scheduled layout"
        );

        let fixed = LayoutStyle {
            position: PositionSpec::Fixed,
            offset_left: Some(LengthSpec::Px(4.0)),
            offset_top: Some(LengthSpec::Px(4.0)),
            width: Some(LengthSpec::Px(28.0)),
            height: Some(LengthSpec::Px(16.0)),
            ..LayoutStyle::default()
        };
        set_layout(&mut world, nid(91), fixed);
        let step = scoped(&mut world, document, viewport, &mut retained);
        assert!(
            step.emitted.contains(&nid(91)),
            "fixed child was not placed: {:?}",
            step.emitted
        );
        assert_flow_not_placed(&step, in_flow);
        assert_eq!(world.layout_box(nid(91)).unwrap().width, 28.0);
    }
    assert_eq!(
        measured[0], measured[1],
        "in-flow measure work grew with sibling count: {measured:?}"
    );
}

/// Growing the containing block places the absolute child that reads it, and
/// does not place a fixed child whose containing block is the viewport.
#[test]
fn containing_block_change_places_dependent_positioned_child() {
    let viewport = LayoutViewport::new(800.0, 600.0);
    let absolute = LayoutStyle {
        position: PositionSpec::Absolute,
        offset_left: Some(LengthSpec::Px(8.0)),
        offset_top: Some(LengthSpec::Px(8.0)),
        width: Some(LengthSpec::Percent(50.0)),
        height: Some(LengthSpec::Px(24.0)),
        ..LayoutStyle::default()
    };
    let (mut world, document) = positioned_world(6, absolute.clone());
    let mut retained = RetainedLayoutCache::default();
    prime(&mut world, document, viewport, &mut retained);
    let before = world.layout_box(nid(90)).unwrap().width;

    let container = LayoutStyle {
        width: Some(LengthSpec::Px(500.0)),
        height: Some(LengthSpec::Px(300.0)),
        direction: Some(FlexDirection::Column),
        align_items: AlignSpec::Start,
        ..LayoutStyle::default()
    };
    set_layout(&mut world, nid(3), container);
    let step = scoped(&mut world, document, viewport, &mut retained);
    assert!(
        step.emitted.contains(&nid(90)),
        "percentage absolute child was not placed when its containing block grew: {:?}",
        step.emitted
    );
    assert!(
        !step.emitted.contains(&nid(91)),
        "fixed child does not depend on the parent's content box: {:?}",
        step.emitted
    );
    assert_flow_not_placed(&step, 6);
    let after = world.layout_box(nid(90)).unwrap().width;
    assert!(
        (after - before).abs() > 1.0,
        "containing block growth did not change the absolute child ({before} -> {after})"
    );
    assert!(
        (after - 250.0).abs() < 0.5,
        "expected 50% of 500, got {after}"
    );
}

/// Repeating an edit of one container replaces its plan. The retained entry
/// count stays with the direct participants and the two measure slots.
#[test]
fn repeated_container_edits_do_not_grow_plan_entries() {
    let viewport = LayoutViewport::new(800.0, 600.0);
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(nid(1), document, NodeKind::Document);
    queue.create(nid(3), document, NodeKind::Element { tag: "div".into() });
    queue.insert(nid(1), nid(3), None);
    queue.set_style(
        nid(3),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(400.0)),
                direction: Some(FlexDirection::Column),
                align_items: AlignSpec::Start,
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for index in 0..6u64 {
        let id = nid(10 + index);
        queue.create(id, document, NodeKind::Element { tag: "div".into() });
        queue.insert(nid(3), id, None);
        queue.set_style(
            id,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
    }
    queue.create(nid(90), document, NodeKind::Element { tag: "div".into() });
    queue.insert(nid(3), nid(90), None);
    queue.set_style(
        nid(90),
        NodeStyle {
            layout: Arc::new(absolute_px(30.0)),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    let mut retained = RetainedLayoutCache::default();
    prime(&mut world, document, viewport, &mut retained);

    let mut counts = Vec::new();
    for step in 0..16 {
        set_layout(
            &mut world,
            nid(10),
            LayoutStyle {
                width: Some(LengthSpec::Px(80.0)),
                height: Some(LengthSpec::Px(20.0 + step as f32)),
                ..LayoutStyle::default()
            },
        );
        let _ = scoped(&mut world, document, viewport, &mut retained);
        counts.push(retained.documents[&document].retained_plan_entries());
        let cache = &retained.documents[&document];
        for plan in cache.container_plans.values() {
            assert!(
                plan.entries.borrow().len() <= plan.children.len(),
                "flow entries exceeded direct children"
            );
            assert!(
                plan.overlay.len() <= plan.children.len(),
                "overlay entries exceeded direct children"
            );
        }
        for slots in cache.measure_plans.values() {
            let occupied = slots.slots.iter().flatten().count();
            assert!(occupied <= 2, "measure slots grew to {occupied}");
            for plan in slots.slots.iter().flatten() {
                assert!(
                    plan.entries.len() <= plan.children.len(),
                    "measure entries exceeded direct children"
                );
            }
        }
    }
    assert_eq!(
        counts[8], counts[15],
        "plan entries kept growing across repeated edits: {counts:?}"
    );
}

/// A badge anchored to the far corners moves when the container it is
/// anchored in grows, though nothing about the badge changed.
#[test]
fn a_far_anchored_badge_follows_its_growing_container() {
    let badge = LayoutStyle {
        position: PositionSpec::Absolute,
        offset_right: Some(LengthSpec::Px(8.0)),
        offset_bottom: Some(LengthSpec::Px(8.0)),
        width: Some(LengthSpec::Px(16.0)),
        height: Some(LengthSpec::Px(16.0)),
        ..LayoutStyle::default()
    };
    let (mut world, document) = positioned_world(3, badge);
    // Content-sized, so a growing row grows the block the badge is in.
    set_layout(
        &mut world,
        nid(3),
        LayoutStyle {
            width: Some(LengthSpec::Px(400.0)),
            direction: Some(FlexDirection::Column),
            align_items: AlignSpec::Start,
            ..LayoutStyle::default()
        },
    );
    let viewport = LayoutViewport::new(800.0, 600.0);
    let mut retained = RetainedLayoutCache::default();
    prime(&mut world, document, viewport, &mut retained);
    let before = world.layout_box(nid(90)).unwrap();
    set_layout(
        &mut world,
        nid(11),
        LayoutStyle {
            height: Some(LengthSpec::Px(60.0)),
            ..px(80.0)
        },
    );
    scoped(&mut world, document, viewport, &mut retained);
    let after = world.layout_box(nid(90)).unwrap();
    assert_eq!(after.y, before.y + 40.0, "{before:?} -> {after:?}");
}
