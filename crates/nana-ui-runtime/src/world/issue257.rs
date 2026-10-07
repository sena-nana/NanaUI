//! Issue 257: a parent constraint measures only children that consume that axis.

use std::sync::{Arc, Mutex};

use nana_text::NativeTextEngine;
use nana_text::font::FontSystem;
use nana_ui_core::{LayoutDependencyFootprint, LayoutStyle, LengthSpec};

use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::{
    LayoutFrontier, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeStyle,
    RetainedLayoutCache, RuntimeLayoutEngine, TextContent,
};

fn node(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

fn document(value: u64) -> DocumentId {
    DocumentId::new(value).unwrap()
}

fn style(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

fn px_box(width: f32, height: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    }
}

fn commit_style(world: &mut UiWorld, id: StableNodeId, layout: LayoutStyle) {
    let mut queue = MutationQueue::new();
    queue.set_style(id, style(layout));
    world.commit(queue).unwrap();
}

fn seeds_after(world: &mut UiWorld) -> Vec<crate::LayoutFrontierSeed> {
    world.take_system_work().layout_frontier_seeds
}

fn frontier_for(world: &UiWorld, seeds: &[crate::LayoutFrontierSeed]) -> LayoutFrontier {
    let graph = world.layout_dependency_graph_for_seeds(document(1), seeds);
    LayoutFrontier::from_dependency_graph(seeds.iter().copied(), &graph)
}

fn section_with_fixed_subtree(fixed_descendants: u64) -> (UiWorld, StableNodeId) {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(1), NodeKind::Document);
    queue.create(
        node(2),
        document(1),
        NodeKind::Element {
            tag: "section".into(),
        },
    );
    queue.insert(node(1), node(2), None);
    queue.set_style(node(2), style(px_box(200.0, 120.0)));

    queue.create(
        node(5),
        document(1),
        NodeKind::Element {
            tag: "fixed".into(),
        },
    );
    queue.insert(node(2), node(5), None);
    queue.set_style(node(5), style(px_box(40.0, 30.0)));
    for offset in 0..fixed_descendants {
        let id = node(100 + offset);
        queue.create(
            id,
            document(1),
            NodeKind::Element {
                tag: "fixed-child".into(),
            },
        );
        queue.insert(node(5), id, None);
        queue.set_style(id, style(px_box(10.0, 10.0)));
    }

    queue.create(
        node(3),
        document(1),
        NodeKind::Element {
            tag: "percent".into(),
        },
    );
    queue.insert(node(2), node(3), None);
    queue.set_style(
        node(3),
        style(LayoutStyle {
            width: Some(LengthSpec::Percent(50.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        }),
    );
    queue.create(
        node(4),
        document(1),
        NodeKind::Element { tag: "fill".into() },
    );
    queue.insert(node(2), node(4), None);
    queue.set_style(
        node(4),
        style(LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Px(20.0)),
            ..LayoutStyle::default()
        }),
    );
    world.commit(queue).unwrap();
    let _ = world.take_system_work();
    (world, node(2))
}

fn fixed_subtree_measure_count(frontier: &LayoutFrontier, fixed_descendants: u64) -> usize {
    let measure = frontier.measure_nodes();
    let mut count = usize::from(measure.contains(&node(5)));
    for offset in 0..fixed_descendants {
        if measure.contains(&node(100 + offset)) {
            count += 1;
        }
    }
    count
}

#[test]
fn parent_width_change_measures_only_children_that_consume_inline() {
    let mut counts = Vec::new();
    for descendants in [4u64, 48] {
        let (mut world, section) = section_with_fixed_subtree(descendants);
        commit_style(&mut world, section, px_box(240.0, 120.0));
        let seeds = seeds_after(&mut world);
        assert!(
            seeds.iter().any(|seed| {
                seed.node == section
                    && seed
                        .invalidation
                        .affected_axes
                        .intersects(LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT)
            }),
            "a width write must publish an inline parent constraint"
        );
        let frontier = frontier_for(&world, &seeds);
        assert!(
            frontier.measure_nodes().contains(&node(3)),
            "a percent child consumes the inline constraint"
        );
        assert!(
            frontier.measure_nodes().contains(&node(4)),
            "a Fill child consumes the inline constraint"
        );
        let fixed = fixed_subtree_measure_count(&frontier, descendants);
        assert_eq!(
            fixed, 0,
            "fixed descendants are not measured ({descendants} nodes, measured {fixed})"
        );
        counts.push(frontier.measure_nodes().len());
    }
    assert_eq!(
        counts[0], counts[1],
        "growing the fixed subtree must not grow the measure frontier: {counts:?}"
    );
}

#[test]
fn width_constraint_relayouts_text_without_reshaping() {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(1), NodeKind::Document);
    queue.create(
        node(2),
        document(1),
        NodeKind::Element {
            tag: "section".into(),
        },
    );
    queue.create(node(3), document(1), NodeKind::Text);
    queue.insert(node(1), node(2), None);
    queue.insert(node(2), node(3), None);
    queue.set_style(node(2), style(px_box(320.0, 80.0)));
    queue.set_style(
        node(3),
        style(LayoutStyle {
            width: Some(LengthSpec::Percent(100.0)),
            ..LayoutStyle::default()
        }),
    );
    queue.set_text(
        node(3),
        TextContent {
            value: "a long wrapping line of text that follows the parent width".into(),
        },
    );
    world.commit(queue).unwrap();

    let engine = Arc::new(Mutex::new(NativeTextEngine::new(FontSystem::hermetic())));
    let mut shaper = NanaTextEngineShaper::new(engine);
    let viewport = LayoutViewport::new(400.0, 200.0);
    let mut retained = RetainedLayoutCache::default();

    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    world.shape_text(&work.text, &mut shaper).unwrap();
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(
            &world,
            document(1),
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            true,
        )
        .unwrap();
    write_boxes(&mut world, &emitted);
    let _ = world.take_system_work();
    world
        .shape_text_for_layout(document(1), &mut shaper)
        .unwrap();

    commit_style(&mut world, node(2), px_box(80.0, 80.0));
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    let shape_before = world
        .nodes
        .text_node(node(3))
        .map(|text| text.revisions.shape);
    let emitted = RuntimeLayoutEngine
        .layout_document_with_frontier(
            &world,
            document(1),
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        )
        .unwrap();
    let text_box = emitted
        .iter()
        .find(|(id, _)| *id == node(3))
        .map(|(_, box_)| *box_)
        .expect("the percent text child is laid out with the new width");
    assert!(
        text_box.width < 100.0,
        "percent width must follow the narrower parent, got {}",
        text_box.width
    );
    write_boxes(&mut world, &emitted);
    let text = world.nodes.text_node(node(3)).expect("text node");
    assert_eq!(text.revisions.shape, shape_before.unwrap());
    assert!(
        text.constraint_moved_without_reshaping(),
        "a width change sets TextDirty::CONSTRAINT and leaves the shaped runs"
    );
    let shaped_before = world.last_work_counters().text_shaped_runs;
    world
        .shape_text_for_layout(document(1), &mut shaper)
        .unwrap();
    let shaped_after = world.last_work_counters().text_shaped_runs;
    let text_work = world.last_text_work_counters();
    assert_eq!(
        shaped_after, shaped_before,
        "a constraint-only width change must not reshape"
    );
    assert_eq!(text_work.text_nodes_shaped, 0);
    assert!(
        text_work.constraint_only_relayouts >= 1,
        "the new width must lay the existing runs out again, got {text_work:?}"
    );
}

fn write_boxes(world: &mut UiWorld, emitted: &[(StableNodeId, crate::LayoutBox)]) {
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

fn layout_with_seeds(
    world: &UiWorld,
    viewport: LayoutViewport,
    seeds: &[crate::LayoutFrontierSeed],
    retained: &mut RetainedLayoutCache,
    force_full: bool,
) -> Vec<(StableNodeId, crate::LayoutBox)> {
    crate::layout_engine::plan_stats::reset();
    RuntimeLayoutEngine
        .layout_document_with_frontier(world, document(1), viewport, seeds, retained, force_full)
        .unwrap()
}

/// Cards in a column that can grow. Each card is a fixed border box around a label.
fn cards(count: u64) -> (UiWorld, StableNodeId, StableNodeId) {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(1), NodeKind::Document);
    queue.create(
        node(2),
        document(1),
        NodeKind::Element {
            tag: "column".into(),
        },
    );
    queue.insert(node(1), node(2), None);
    queue.set_style(
        node(2),
        style(LayoutStyle {
            width: Some(LengthSpec::Px(240.0)),
            direction: Some(nana_ui_core::FlexDirection::Column),
            ..LayoutStyle::default()
        }),
    );
    for index in 0..count {
        let card = node(1_000 + index * 2);
        let label = node(1_001 + index * 2);
        queue.create(card, document(1), NodeKind::Element { tag: "card".into() });
        queue.create(label, document(1), NodeKind::Text);
        queue.insert(node(2), card, None);
        queue.insert(card, label, None);
        queue.set_style(card, style(px_box(120.0, 32.0)));
        queue.set_text(label, TextContent { value: "Ok".into() });
    }
    world.commit(queue).unwrap();
    let _ = world.take_system_work();
    let edited = count / 2;
    (world, node(1_000 + edited * 2), node(1_001 + edited * 2))
}

fn shape_document(world: &mut UiWorld, shaper: &mut NanaTextEngineShaper) {
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    world.shape_text(&work.text, shaper).unwrap();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExecutionDelta {
    measure_nodes: usize,
    measure_cache_hits: usize,
    measure_cache_misses: usize,
    placement_nodes: usize,
    origin_only_updates: usize,
}

fn execution_delta(
    before: nana_ui_core::WorkCounters,
    after: nana_ui_core::WorkCounters,
) -> ExecutionDelta {
    ExecutionDelta {
        measure_nodes: after
            .layout_measure_nodes
            .saturating_sub(before.layout_measure_nodes),
        measure_cache_hits: after
            .layout_measure_cache_hits
            .saturating_sub(before.layout_measure_cache_hits),
        measure_cache_misses: after
            .layout_measure_cache_misses
            .saturating_sub(before.layout_measure_cache_misses),
        placement_nodes: after
            .layout_placement_nodes
            .saturating_sub(before.layout_placement_nodes),
        origin_only_updates: after
            .layout_origin_only_updates
            .saturating_sub(before.layout_origin_only_updates),
    }
}

struct GateACounts {
    ancestor_measure: usize,
    sibling_placement: usize,
    full_document_fallbacks: usize,
    card_measured: bool,
    metric_generation_bumps: usize,
    execution: ExecutionDelta,
}

fn contained_label_edit(count: u64) -> GateACounts {
    let engine = Arc::new(Mutex::new(NativeTextEngine::new(FontSystem::hermetic())));
    let mut shaper = NanaTextEngineShaper::new(engine);
    let viewport = LayoutViewport::new(320.0, 800.0);
    let (mut world, card, label) = cards(count);
    shape_document(&mut world, &mut shaper);
    let mut retained = RetainedLayoutCache::default();
    let emitted = layout_with_seeds(&world, viewport, &[], &mut retained, true);
    write_boxes(&mut world, &emitted);
    let _ = retained.take_intrinsic_counters();
    let _ = world.take_system_work();
    shape_document(&mut world, &mut shaper);
    let _ = world.take_system_work();

    let mut queue = MutationQueue::new();
    queue.set_text(
        label,
        TextContent {
            value: "Longer label".into(),
        },
    );
    world.commit(queue).unwrap();
    let pending = world.take_system_work();
    world.resolve_styles(&pending.style).unwrap();
    world.shape_text(&pending.text, &mut shaper).unwrap();
    let seeded = world.take_system_work();
    let frontier = frontier_for(&world, &seeded.layout_frontier_seeds);
    let before = world.last_work_counters();
    layout_with_seeds(
        &world,
        viewport,
        &seeded.layout_frontier_seeds,
        &mut retained,
        false,
    );
    let stats = retained.frontier_stats(document(1));
    world.record_layout_frontier(stats);
    world.record_layout_execution(retained.execution_stats(document(1)));
    let metric_generation_bumps = retained.take_intrinsic_counters().generation_bumps;
    let after = world.last_work_counters();
    assert_eq!(
        after.layout_full_document_fallbacks - before.layout_full_document_fallbacks,
        stats.full_document_fallbacks as usize,
        "frontier stats are the work-counter authority"
    );
    assert_eq!(
        world.layout_box(card).map(|box_| (box_.width, box_.height)),
        Some((120.0, 32.0)),
        "the fixed card's exported size stays 120x32"
    );
    let mut sibling_placement = 0usize;
    for index in 0..count {
        let other = node(1_000 + index * 2);
        if other == card {
            continue;
        }
        if frontier.placement_nodes().contains(&other) {
            sibling_placement += 1;
        }
    }
    let ancestor_measure = usize::from(frontier.measure_nodes().contains(&node(1)))
        + usize::from(frontier.measure_nodes().contains(&node(2)));
    GateACounts {
        ancestor_measure,
        sibling_placement,
        full_document_fallbacks: stats.full_document_fallbacks,
        card_measured: frontier.measure_nodes().contains(&card),
        metric_generation_bumps,
        execution: execution_delta(before, after),
    }
}

#[test]
fn fixed_card_label_edit_does_not_measure_ancestors_or_place_siblings() {
    let mut counts = Vec::new();
    for count in [48u64, 160] {
        let gate = contained_label_edit(count);
        assert_eq!(
            gate.ancestor_measure, 0,
            "{count} cards: ancestors entered the measure frontier"
        );
        assert!(
            !gate.card_measured,
            "{count} cards: the fixed card was remeasured for an inner label"
        );
        assert_eq!(
            gate.sibling_placement, 0,
            "{count} cards: siblings entered placement"
        );
        assert_eq!(gate.full_document_fallbacks, 0);
        counts.push((
            gate.ancestor_measure,
            gate.sibling_placement,
            gate.full_document_fallbacks,
            gate.metric_generation_bumps,
            gate.execution,
        ));
        assert!(
            gate.execution.measure_nodes + gate.execution.placement_nodes > 0,
            "{count} cards: the edit produced no measure or placement work"
        );
        assert!(
            gate.metric_generation_bumps > 0,
            "{count} cards: the label's new metrics did not bump generation"
        );
    }
    assert_eq!(
        counts[0], counts[1],
        "structural counts must be constant as the card list grows: {counts:?}"
    );
}

fn rows(count: u64) -> UiWorld {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(1), NodeKind::Document);
    queue.create(
        node(2),
        document(1),
        NodeKind::Element {
            tag: "column".into(),
        },
    );
    queue.insert(node(1), node(2), None);
    queue.set_style(
        node(2),
        style(LayoutStyle {
            width: Some(LengthSpec::Px(200.0)),
            direction: Some(nana_ui_core::FlexDirection::Column),
            ..LayoutStyle::default()
        }),
    );
    for index in 0..count {
        let row = node(3_000 + index);
        queue.create(row, document(1), NodeKind::Element { tag: "row".into() });
        queue.insert(node(2), row, None);
        queue.set_style(row, style(px_box(180.0, 20.0)));
    }
    world.commit(queue).unwrap();
    let _ = world.take_system_work();
    world
}

struct GateBCounts {
    children_measured: usize,
    suffixes_replayed: usize,
    following_measured: usize,
    prefix_emitted: bool,
    full_document_fallbacks: usize,
    execution: ExecutionDelta,
}

fn late_row_height(count: u64) -> GateBCounts {
    let viewport = LayoutViewport::new(240.0, 400.0);
    let mut world = rows(count);
    let edited_index = count.saturating_sub(8);
    let following = node(3_000 + edited_index + 1);
    let following_label = node(4_000 + edited_index);
    let mut queue = MutationQueue::new();
    queue.create(following_label, document(1), NodeKind::Text);
    queue.insert(following, following_label, None);
    queue.set_style(following_label, style(px_box(40.0, 12.0)));
    queue.set_text(
        following_label,
        TextContent {
            value: "tail".into(),
        },
    );
    world.commit(queue).unwrap();
    let _ = world.take_system_work();
    let mut retained = RetainedLayoutCache::default();
    let emitted = layout_with_seeds(&world, viewport, &[], &mut retained, true);
    write_boxes(&mut world, &emitted);
    let _ = world.take_system_work();

    // The full pass records no measure plan. One scoped edit builds it.
    let primer = node(3_000);
    let mut primed = (*world.node_style(primer).unwrap().layout).clone();
    primed.height = Some(LengthSpec::Px(21.0));
    commit_style(&mut world, primer, primed);
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    let emitted = layout_with_seeds(
        &world,
        viewport,
        &work.layout_frontier_seeds,
        &mut retained,
        false,
    );
    write_boxes(&mut world, &emitted);
    let _ = world.take_system_work();

    let edited = node(3_000 + edited_index);
    let mut layout = (*world.node_style(edited).unwrap().layout).clone();
    layout.height = Some(LengthSpec::Px(36.0));
    commit_style(&mut world, edited, layout);
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    let frontier = frontier_for(&world, &work.layout_frontier_seeds);
    let before = world.last_work_counters();
    let emitted = layout_with_seeds(
        &world,
        viewport,
        &work.layout_frontier_seeds,
        &mut retained,
        false,
    );
    let stats = retained.frontier_stats(document(1));
    world.record_layout_frontier(stats);
    world.record_layout_execution(retained.execution_stats(document(1)));
    let execution = execution_delta(before, world.last_work_counters());
    let following_measured = (edited_index + 1..count)
        .filter(|index| frontier.measure_nodes().contains(&node(3_000 + index)))
        .count();
    assert!(
        frontier.placement_nodes().contains(&following),
        "the next row stays on the placement frontier"
    );
    assert!(
        frontier.placement_nodes().contains(&following_label),
        "a descendant that only moves stays on placement"
    );
    assert!(!frontier.measure_nodes().contains(&following_label));
    assert!(!frontier.measure_nodes().contains(&node(3_000)));
    assert!(!frontier.placement_nodes().contains(&node(3_000)));
    let prefix_emitted = emitted.iter().any(|(id, _)| *id == node(3_000));
    GateBCounts {
        children_measured: crate::layout_engine::plan_stats::children_measured(),
        suffixes_replayed: crate::layout_engine::plan_stats::suffixes_replayed(),
        following_measured,
        prefix_emitted,
        full_document_fallbacks: stats.full_document_fallbacks,
        execution,
    }
}

#[test]
fn late_row_height_replays_suffix_without_measuring_followers() {
    let mut measured = Vec::new();
    for count in [64u64, 256] {
        let gate = late_row_height(count);
        assert_eq!(
            gate.following_measured, 0,
            "{count} rows: following siblings entered the measure frontier"
        );
        assert!(
            gate.suffixes_replayed > 0,
            "{count} rows: the column did not replay its suffix"
        );
        assert!(
            !gate.prefix_emitted,
            "{count} rows: prefix placement was rebuilt"
        );
        assert_eq!(gate.full_document_fallbacks, 0);
        assert!(
            gate.children_measured <= 4,
            "{count} rows: unaffected children were measured ({})",
            gate.children_measured
        );
        assert!(
            gate.execution.origin_only_updates > 0,
            "{count} rows: the suffix move did not record an origin-only placement"
        );
        assert!(
            gate.execution.placement_nodes > 0,
            "{count} rows: placement wrote no boxes"
        );
        measured.push((gate.children_measured, gate.execution));
    }
    assert_eq!(
        measured[0], measured[1],
        "measure work must not grow with the row count: {measured:?}"
    );
}

#[test]
fn equivalent_recompute_does_not_publish_layout_geometry() {
    let mut world = UiWorld::new();
    let mut queue = MutationQueue::new();
    queue.create(node(1), document(1), NodeKind::Document);
    queue.create(
        node(2),
        document(1),
        NodeKind::Element {
            tag: "section".into(),
        },
    );
    queue.create(
        node(3),
        document(1),
        NodeKind::Element {
            tag: "fixed".into(),
        },
    );
    queue.insert(node(1), node(2), None);
    queue.insert(node(1), node(3), None);
    queue.set_style(node(2), style(px_box(120.0, 32.0)));
    queue.set_style(
        node(3),
        style(LayoutStyle {
            position: nana_ui_core::PositionSpec::Fixed,
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(16.0)),
            offset_top: Some(LengthSpec::Px(4.0)),
            offset_left: Some(LengthSpec::Px(6.0)),
            ..LayoutStyle::default()
        }),
    );
    world.commit(queue).unwrap();
    let _ = world.take_system_work();
    let viewport = LayoutViewport::new(200.0, 100.0);
    let mut retained = RetainedLayoutCache::default();
    let emitted = layout_with_seeds(&world, viewport, &[], &mut retained, true);
    write_boxes(&mut world, &emitted);
    let _ = retained.take_intrinsic_counters();
    let _ = world.take_system_work();
    let ids = emitted.iter().map(|(id, _)| *id).collect::<Vec<_>>();
    world.publish_layout_results(&ids, crate::LayoutResultSource::RuntimeLayout);
    let generation = world.layout_generation();
    let section_ptr = world
        .layout_results
        .get(&node(2))
        .map(Arc::as_ptr)
        .expect("section result");
    let fixed_ptr = world
        .layout_results
        .get(&node(3))
        .map(Arc::as_ptr)
        .expect("fixed result");

    // A definite min-width below the used width recomputes the section and
    // leaves the border box unchanged.
    let mut layout = (*world.node_style(node(2)).unwrap().layout).clone();
    layout.min_width = Some(LengthSpec::Px(10.0));
    commit_style(&mut world, node(2), layout);
    let work = world.take_system_work();
    world.resolve_styles(&work.style).unwrap();
    let emitted = layout_with_seeds(
        &world,
        viewport,
        &work.layout_frontier_seeds,
        &mut retained,
        false,
    );
    assert_eq!(
        retained.take_intrinsic_counters().generation_bumps,
        0,
        "a bit-equivalent recompute must not bump retained metric generation"
    );
    write_boxes(&mut world, &emitted);
    let _ = world.take_system_work();
    let ids = emitted.iter().map(|(id, _)| *id).collect::<Vec<_>>();
    let before = world.layout_box(node(2));
    let counters_before = world.last_work_counters();
    world.publish_layout_results(&ids, crate::LayoutResultSource::RuntimeLayout);
    let counters_after = world.last_work_counters();

    assert_eq!(
        (world.layout_generation(), world.layout_box(node(2))),
        (generation, before),
        "a bit-equivalent recompute must not bump layout generation"
    );
    assert_eq!(
        world.layout_results.get(&node(2)).map(Arc::as_ptr),
        Some(section_ptr),
        "the section result object must stay the one already published"
    );
    assert_eq!(
        world.layout_results.get(&node(3)).map(Arc::as_ptr),
        Some(fixed_ptr),
        "an untouched fixed-position result must not be republished"
    );
    assert!(world.layout_result(node(2)).is_some());
    assert_eq!(world.layout_result(node(2)).unwrap().bounds.width, 120.0);
    assert_eq!(
        counters_after.layout_result_changed, counters_before.layout_result_changed,
        "a bit-equivalent recompute must not count a changed layout result"
    );
    assert_eq!(
        counters_after.layout_delta_commits, counters_before.layout_delta_commits,
        "a bit-equivalent recompute must not commit a layout delta"
    );
    assert!(
        counters_after.layout_result_reused > counters_before.layout_result_reused,
        "keeping the published object must count as reuse"
    );

    // position:fixed always misses the cheap geometry check. Publishing the
    // same boxes again must still keep the Arc.
    let fixed_before = world.last_work_counters();
    world.publish_layout_results(&[node(3)], crate::LayoutResultSource::RuntimeLayout);
    let fixed_after = world.last_work_counters();
    assert_eq!(world.layout_generation(), generation);
    assert_eq!(
        world.layout_results.get(&node(3)).map(Arc::as_ptr),
        Some(fixed_ptr)
    );
    assert_eq!(
        fixed_after.layout_result_changed,
        fixed_before.layout_result_changed
    );
    assert_eq!(
        fixed_after.layout_delta_commits,
        fixed_before.layout_delta_commits
    );
    assert!(fixed_after.layout_result_reused > fixed_before.layout_result_reused);
}
