//! Issue #258 inline formatting-context scope.
//!
//! A fixed-size atomic keeps its line. An intrinsic width change reflows only
//! that inline formatting context.

use std::sync::Arc;

use nana_ui_core::{AlignSpec, DisplaySpec, FlexDirection, LayoutStyle, LengthSpec};

use crate::{DocumentId, MutationQueue, NodeKind, NodeStyle, TextContent, UiWorld};

use super::*;

fn id(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

fn column_style() -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::Flex),
        direction: Some(FlexDirection::Column),
        width: Some(LengthSpec::Px(400.0)),
        height: Some(LengthSpec::Px(800.0)),
        align_items: AlignSpec::Start,
        justify_content: nana_ui_core::JustifySpec::Start,
        ..LayoutStyle::default()
    }
}

fn ifc_style(padding_top: Option<f32>) -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::Block),
        width: Some(LengthSpec::Px(200.0)),
        padding_top: padding_top.map(LengthSpec::Px),
        ..LayoutStyle::default()
    }
}

fn atomic(width: f32) -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::InlineBlock),
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(24.0)),
        ..LayoutStyle::default()
    }
}

fn block_box(width: f32, height: f32) -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::Block),
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    }
}

fn inline_text() -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::Inline),
        ..LayoutStyle::default()
    }
}

struct Step {
    emitted: Vec<StableNodeId>,
    children_measured: usize,
    measure_nodes: usize,
    full_subtrees: usize,
    full_document_fallbacks: usize,
}

fn write_boxes(world: &mut UiWorld, emitted: &[(StableNodeId, LayoutBox)]) {
    let mut queue = MutationQueue::new();
    let mut wrote = false;
    for (id, box_) in emitted {
        if world.layout_box(*id) != Some(*box_) {
            queue.write_layout(*id, *box_);
            wrote = true;
        }
    }
    if wrote {
        world.commit(queue).unwrap();
    }
}

fn scoped(
    world: &mut UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
    retained: &mut RetainedLayoutCache,
) -> Step {
    let before = retained
        .documents
        .get(&document)
        .map(|cache| cache.intrinsic_counters.intrinsic_measure_full_subtrees)
        .unwrap_or(0);
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    plan_stats::reset();
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
        children_measured: plan_stats::children_measured(),
        measure_nodes: cache.execution_stats.measure_nodes,
        full_subtrees: cache
            .intrinsic_counters
            .intrinsic_measure_full_subtrees
            .saturating_sub(before),
        full_document_fallbacks: plan_stats::full_document_fallbacks(),
    };
    write_boxes(world, &emitted);
    let _ = world.take_system_work();
    let expected = RuntimeLayoutEngine
        .layout_document(world, document, viewport)
        .unwrap();
    let cached = &retained.documents[&document].boxes;
    for (node, box_) in &expected {
        assert_eq!(
            cached.get(node).copied().unwrap_or_default(),
            *box_,
            "scoped layout diverged at {node:?}"
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
    let victim = id(10);
    let original = world.node_style(victim).unwrap().layout.clone();
    let mut nudged = (*original).clone();
    nudged.margin_top = Some(LengthSpec::Px(3.0));
    for layout in [Arc::new(nudged), original] {
        let mut queue = MutationQueue::new();
        queue.set_style(
            victim,
            NodeStyle {
                layout,
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        let _ = scoped(world, document, viewport, retained);
    }
}

fn prepare(
    world: &mut UiWorld,
    document: DocumentId,
    viewport: LayoutViewport,
    retained: &mut RetainedLayoutCache,
) {
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    world
        .shape_text(&[id(300), id(500)], &mut crate::MeasureTextShaper)
        .unwrap();
    let _ = world.take_system_work();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(world, document, viewport, &[], retained, true)
        .unwrap();
    write_boxes(world, &emitted);
    let _ = world.take_system_work();
    prime(world, document, viewport, retained);
}

fn set_style(world: &mut UiWorld, node: StableNodeId, layout: LayoutStyle) {
    let mut queue = MutationQueue::new();
    queue.set_style(
        node,
        NodeStyle {
            layout: Arc::new(layout),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
}

/// Two block containers of inline-blocks. Each is its own inline formatting
/// context. Items are 80px in a 200px line, so they pair onto lines.
fn two_contexts(
    lines: usize,
    other_lines: usize,
    inner: bool,
    padding_top: Option<f32>,
) -> (UiWorld, DocumentId) {
    let document = DocumentId::new(1).unwrap();
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.create(id(3), document, NodeKind::Element { tag: "div".into() });
    queue.create(id(4), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.insert(id(2), id(3), None);
    queue.insert(id(2), id(4), None);
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(column_style()),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(3),
        NodeStyle {
            layout: Arc::new(ifc_style(padding_top)),
            ..NodeStyle::default()
        },
    );
    queue.set_style(
        id(4),
        NodeStyle {
            layout: Arc::new(ifc_style(None)),
            ..NodeStyle::default()
        },
    );
    for index in 0..lines * 2 {
        let node = id(10 + index as u64);
        queue.create(node, document, NodeKind::Element { tag: "span".into() });
        queue.insert(id(3), node, None);
        queue.set_style(
            node,
            NodeStyle {
                layout: Arc::new(atomic(80.0)),
                ..NodeStyle::default()
            },
        );
    }
    if inner {
        queue.create(id(9), document, NodeKind::Element { tag: "div".into() });
        queue.insert(id(10), id(9), None);
        queue.set_style(
            id(9),
            NodeStyle {
                layout: Arc::new(block_box(20.0, 10.0)),
                ..NodeStyle::default()
            },
        );
    }
    queue.create(id(300), document, NodeKind::Text);
    queue.insert(id(12), id(300), None);
    queue.set_text(
        id(300),
        TextContent {
            value: "line-two".into(),
        },
    );
    queue.set_style(
        id(300),
        NodeStyle {
            layout: Arc::new(inline_text()),
            ..NodeStyle::default()
        },
    );
    for index in 0..other_lines * 2 {
        let node = id(400 + index as u64);
        queue.create(node, document, NodeKind::Element { tag: "span".into() });
        queue.insert(id(4), node, None);
        queue.set_style(
            node,
            NodeStyle {
                layout: Arc::new(atomic(80.0)),
                ..NodeStyle::default()
            },
        );
    }
    queue.create(id(500), document, NodeKind::Text);
    queue.insert(id(400), id(500), None);
    queue.set_text(
        id(500),
        TextContent {
            value: "other-context".into(),
        },
    );
    queue.set_style(
        id(500),
        NodeStyle {
            layout: Arc::new(inline_text()),
            ..NodeStyle::default()
        },
    );
    world.commit(queue).unwrap();
    (world, document)
}

fn box_y(world: &UiWorld, node: StableNodeId) -> f32 {
    world.layout_box(node).expect("laid out").y
}

/// A content change inside a fixed inline-block does not reflow its line or
/// the other inline formatting context.
#[test]
fn inline_fixed_size_atomic_does_not_reflow_its_line() {
    let viewport = LayoutViewport::new(800.0, 600.0);
    let mut measured = Vec::new();
    for other_lines in [2usize, 6] {
        let (mut world, document) = two_contexts(2, other_lines, true, None);
        let mut retained = RetainedLayoutCache::default();
        prepare(&mut world, document, viewport, &mut retained);
        let line_y = box_y(&world, id(12));
        let other_y = box_y(&world, id(400));

        set_style(&mut world, id(9), block_box(40.0, 10.0));
        let step = scoped(&mut world, document, viewport, &mut retained);
        assert_eq!(step.full_document_fallbacks, 0);
        assert!(
            !step.emitted.contains(&id(3)),
            "the parent inline context was reflowed: {:?}",
            step.emitted
        );
        assert!(
            !step.emitted.contains(&id(4)) && !step.emitted.contains(&id(400)),
            "the other inline context was considered: {:?}",
            step.emitted
        );
        assert!((box_y(&world, id(12)) - line_y).abs() < 0.01);
        assert!((box_y(&world, id(400)) - other_y).abs() < 0.01);
        assert_eq!(world.layout_box(id(10)).unwrap().width, 80.0);
        measured.push(step.children_measured);
    }
    assert_eq!(
        measured[0], measured[1],
        "the other inline context added measure work: {measured:?}"
    );
}

/// Widening one atomic reflows its formatting context only. The other context
/// is not measured, unaffected lines are not fully remeasured, and unrelated
/// text is not reshaped.
#[test]
fn inline_intrinsic_width_change_reflows_only_its_context() {
    let viewport = LayoutViewport::new(800.0, 600.0);
    let mut measured = Vec::new();
    for lines in [2usize, 6] {
        let (mut world, document) = two_contexts(lines, 6, false, None);
        let mut retained = RetainedLayoutCache::default();
        prepare(&mut world, document, viewport, &mut retained);
        let line_y = box_y(&world, id(12));

        set_style(&mut world, id(10), atomic(100.0));
        let step = scoped(&mut world, document, viewport, &mut retained);
        assert_eq!(step.full_document_fallbacks, 0);
        assert_eq!(
            step.full_subtrees, 0,
            "{lines} lines fully remeasured an unaffected line"
        );
        assert!(
            !step.emitted.contains(&id(400)) && !step.emitted.contains(&id(4)),
            "the other inline context was measured: {:?}",
            step.emitted
        );
        assert_eq!(world.layout_box(id(10)).unwrap().width, 100.0);
        assert!(
            (box_y(&world, id(12)) - line_y).abs() < 0.01,
            "the next line moved to {}",
            box_y(&world, id(12))
        );
        let shaped_before = world.last_work_counters().text_shaped_runs;
        world
            .shape_text(&[id(300), id(500)], &mut crate::MeasureTextShaper)
            .unwrap();
        assert_eq!(
            world.last_work_counters().text_shaped_runs,
            shaped_before,
            "unrelated text runs were reshaped"
        );
        measured.push(step.measure_nodes);
    }
    assert_eq!(
        measured[0], measured[1],
        "unaffected lines added measure work: {measured:?}"
    );

    let mut other_measured = Vec::new();
    for other_lines in [2usize, 6] {
        let (mut world, document) = two_contexts(2, other_lines, false, None);
        let mut retained = RetainedLayoutCache::default();
        prepare(&mut world, document, viewport, &mut retained);
        set_style(&mut world, id(10), atomic(100.0));
        let step = scoped(&mut world, document, viewport, &mut retained);
        assert_eq!(step.full_document_fallbacks, 0);
        assert!(!step.emitted.contains(&id(4)));
        other_measured.push(step.children_measured);
    }
    assert_eq!(
        other_measured[0], other_measured[1],
        "the other inline context was measured as it grew: {other_measured:?}"
    );
}

/// Padding makes the line break non-local. The fallback stays inside that
/// inline formatting context.
#[test]
fn inline_line_break_falls_back_to_its_formatting_context_only() {
    let viewport = LayoutViewport::new(800.0, 600.0);
    let (mut world, document) = two_contexts(2, 4, false, Some(4.0));
    let mut retained = RetainedLayoutCache::default();
    prepare(&mut world, document, viewport, &mut retained);
    set_style(&mut world, id(10), atomic(100.0));
    let step = scoped(&mut world, document, viewport, &mut retained);
    assert_eq!(step.full_document_fallbacks, 0);
    assert!(
        step.full_subtrees > 0,
        "the formatting context was not remeasured"
    );
    assert!(
        !step.emitted.contains(&id(4)) && !step.emitted.contains(&id(400)),
        "the fallback left its formatting context: {:?}",
        step.emitted
    );
}

/// A relatively offset fixed inline-block keeps its offset, once, when what
/// is inside it changes; and a percentage padding on its context resolves
/// against that context's own containing block.
#[test]
fn a_relative_fixed_atomic_keeps_its_offset_across_inner_edits() {
    let viewport = LayoutViewport::new(800.0, 600.0);
    let (mut world, document) = two_contexts(2, 1, true, None);
    set_style(
        &mut world,
        id(3),
        LayoutStyle {
            padding_left: Some(LengthSpec::Percent(10.0)),
            ..ifc_style(None)
        },
    );
    set_style(
        &mut world,
        id(10),
        LayoutStyle {
            position: nana_ui_core::PositionSpec::Relative,
            offset_left: Some(LengthSpec::Px(10.0)),
            offset_top: Some(LengthSpec::Px(3.0)),
            ..atomic(80.0)
        },
    );
    let mut retained = RetainedLayoutCache::default();
    prepare(&mut world, document, viewport, &mut retained);
    let before = world.layout_box(id(10)).unwrap();
    for width in [30.0, 40.0, 50.0] {
        set_style(&mut world, id(9), block_box(width, 10.0));
        scoped(&mut world, document, viewport, &mut retained);
        assert_eq!(world.layout_box(id(10)).unwrap(), before, "after {width}");
    }
}
