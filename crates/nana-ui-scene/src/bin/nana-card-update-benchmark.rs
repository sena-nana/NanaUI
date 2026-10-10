//! Issue #228: what an application's list refresh costs, written the way
//! applications write it.
//!
//! `nana-dirty-frame-benchmark` drives raw mutation queues on a flat list and
//! never parks anything. A real card is built from components — a `ListItem`
//! per row with a leading `Thumbnail` and a trailing row of a `Switch` and two
//! `Button`s — refreshed by rewriting every row through `update_component`,
//! and it always has parked nodes somewhere (a row without a picture parks its
//! thumbnail). Both of those were outside every earlier measurement.
//!
//! Three operations per frame:
//!
//! - `noop`: rewrite every component of every row with the values it already
//!   has, then flush. What a refresh that changed nothing costs.
//! - `select`: move the selection between the last two rows, then flush.
//! - `label`: lengthen or shorten the last row's label, then flush.
//!
//! Two sizes are swept independently: `--rows` (rows in the card) and
//! `--filler` (unrelated nodes elsewhere in the document). The work a one-row
//! change owes depends on neither; anything that grows with `--filler` is
//! whole-document work. `--parked off` gives every row a picture, so nothing
//! in the world is parked — the shape the older benchmark measured.
//!
//! `--append off` drops NanaLive's `append_child(item, thumb)` before
//! `set_list_item_slots`. The slots call inserts and orders the slot nodes
//! itself, so that append only moves the thumbnail to the end for the slots
//! call to move it back: two real moves per row per refresh.

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_ui_core::ButtonKind;
use nana_ui_runtime::view::{AnyView, EntityRef, IntoView, column, entity_ref, widget, with_refs};
use nana_ui_runtime::{
    Button, DocumentId, Entity, FrameStage, LayoutViewport, List, ListItem, ListItemSlots,
    MeasureTextShaper, Stack, StageStatus, Switch, Text, Thumbnail,
};
use nana_ui_scene::RuntimeDocument;
use serde::Serialize;

const DOCUMENT: u64 = 1;
/// Containers between the document root and the card, like a shell, a dock
/// and a page. Ancestor walks are paid per level.
const DEPTH: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Noop,
    Select,
    Label,
}

impl Operation {
    const ALL: [(Self, &'static str); 3] = [
        (Self::Noop, "noop"),
        (Self::Select, "select"),
        (Self::Label, "label"),
    ];

    fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find_map(|(operation, name)| (name == raw).then_some(operation))
    }

    fn name(self) -> &'static str {
        Self::ALL
            .into_iter()
            .find_map(|(operation, name)| (operation == self).then_some(name))
            .expect("every operation is named")
    }
}

struct Row {
    item: Entity<ListItem>,
    thumb: Entity<Thumbnail>,
    trail: Entity<Stack>,
    switch: Entity<Switch>,
    buttons: [Entity<Button>; 2],
}

struct Card {
    runtime: RuntimeDocument,
    rows: Vec<Row>,
    /// Row state the application renders from.
    labels: Vec<String>,
    selected: usize,
    parked: bool,
    append: bool,
}

fn has_picture(parked: bool, row: usize) -> bool {
    !parked || row.is_multiple_of(2)
}

fn build(rows: usize, filler: usize, parked: bool, append: bool) -> Card {
    let document = DocumentId::new(DOCUMENT).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    /// A row's refs: item, thumbnail, trailing stack, switch, buttons.
    type RowRefs = (
        EntityRef<ListItem>,
        EntityRef<Thumbnail>,
        EntityRef<Stack>,
        EntityRef<Switch>,
        [EntityRef<Button>; 2],
    );
    fn nest(depth: usize, rows: usize, filler: usize, refs: &mut Vec<RowRefs>) -> AnyView {
        if depth > 0 {
            return column()
                .children(nest(depth - 1, rows, filler, refs))
                .into_any();
        }
        let fillers = (0..filler)
            .map(|index| widget(Text::new(format!("filler {index}"))))
            .collect::<Vec<_>>();
        let items = (0..rows)
            .map(|index| {
                let row: RowRefs = (
                    entity_ref(),
                    entity_ref(),
                    entity_ref(),
                    entity_ref(),
                    [entity_ref(), entity_ref()],
                );
                refs.push(row);
                widget(ListItem::new(format!("Motion {index}")))
                    .entity_ref(row.0)
                    .children((
                        widget(Thumbnail::new(format!("slot-{index}"))).entity_ref(row.1),
                        widget(Stack::row(4.0)).entity_ref(row.2).children((
                            widget(Switch::new("", false)).entity_ref(row.3),
                            widget(Button::new("收藏")).entity_ref(row.4[0]),
                            widget(Button::new("按键")).entity_ref(row.4[1]),
                        )),
                    ))
            })
            .collect::<Vec<_>>();
        (
            column().children(fillers),
            widget(List::new()).children(items),
        )
            .into_any()
    }
    let (_, built) = runtime
        .context_mut()
        .mount_view_root(document, || {
            let mut refs = Vec::with_capacity(rows);
            let view = nest(DEPTH, rows, filler, &mut refs);
            with_refs(view, refs)
        })
        .unwrap();
    let built = built
        .into_iter()
        .map(|(item, thumb, trail, switch, buttons)| Row {
            item,
            thumb,
            trail,
            switch,
            buttons,
        })
        .collect();
    let mut card = Card {
        runtime,
        rows: built,
        labels: (0..rows).map(|index| format!("Motion {index}")).collect(),
        selected: 0,
        parked,
        append,
    };
    paint(&mut card);
    card
}

/// One application refresh: every row, every component, current values.
///
/// This is NanaLive's `paint_card_items` without its row fingerprint — the
/// fingerprint is the workaround Issue #228 asks the framework to make
/// unnecessary.
fn paint(card: &mut Card) {
    let context = card.runtime.context_mut();
    for (index, row) in card.rows.iter().enumerate() {
        let label = card.labels[index].clone();
        let selected = card.selected == index;
        context
            .update_component(row.item, |item, _| {
                item.label = label.clone();
                item.detail = String::new();
                item.disabled = false;
                item.selected = selected;
                Arc::make_mut(&mut item.style.layout).hidden = false;
            })
            .unwrap();
        let picture = has_picture(card.parked, index);
        if picture {
            let slot = format!("slot-{index}");
            context
                .update_component(row.thumb, |thumb, _| {
                    *thumb = Thumbnail::new(slot.as_str()).label(label.as_str());
                })
                .unwrap();
            if card.append {
                context.append_child(row.item, row.thumb).unwrap();
            }
        } else {
            context
                .update_component(row.thumb, |_, cx| {
                    let id = cx.entity().stable_id();
                    cx.mutations().park_subtree(id);
                })
                .unwrap();
        }
        context
            .update_component(row.switch, |switch, _| {
                switch.checked = index.is_multiple_of(3);
                switch.disabled = false;
                Arc::make_mut(&mut switch.style.layout).hidden = false;
            })
            .unwrap();
        for (button, label) in row.buttons.iter().zip(["收藏", "按键"]) {
            context
                .update_component(*button, |button, _| {
                    button.label = label.into();
                    button.kind = ButtonKind::Ghost;
                    button.disabled = false;
                    Arc::make_mut(&mut button.style.layout).hidden = false;
                })
                .unwrap();
        }
        context
            .set_list_item_slots(
                row.item,
                ListItemSlots {
                    leading: picture.then(|| row.thumb.stable_id()),
                    content: None,
                    trailing: Some(row.trail.stable_id()),
                },
            )
            .unwrap();
    }
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    phase: &'static str,
    samples: usize,
    warmup: usize,
    cells: Vec<Cell>,
}

#[derive(Serialize)]
struct Cell {
    operation: &'static str,
    rows: usize,
    filler: usize,
    parked: bool,
    append: bool,
    nodes: usize,
    write_ms: Stat,
    flush_ms: Stat,
    /// Generation bumps the writes caused, per frame (mean).
    generations: f64,
    /// Frames the flush found nothing to do.
    idle_flushes: usize,
    stages_p50_ms: Vec<(String, f64)>,
    children_measured: f64,
    plans_reused: f64,
    layout_nodes: usize,
    layout_frontier_seeds: usize,
    layout_frontier_seed_merges: usize,
    layout_frontier_nodes_measure: usize,
    layout_frontier_nodes_placement: usize,
    layout_frontier_contexts: usize,
    layout_dependency_edges_visited: usize,
    layout_propagations_stopped: usize,
    layout_local_subtree_fallbacks: usize,
    layout_full_document_fallbacks: usize,
    layout_suffixes_replayed: usize,
    layout_children_measured: usize,
    layout_containers_uncacheable: usize,
    layout_result_children_visited: usize,
    layout_placement_plans_reused: usize,
    layout_measure_plans_reused: usize,
    layout_plan_misses: usize,
    layout_plan_rebuilds: usize,
    layout_scratch_entries: usize,
    layout_scratch_bytes: usize,
    intrinsic_measure_requests: usize,
    intrinsic_measure_full_subtrees: usize,
    constraint_change_seeds: usize,
    constraint_dependents_considered: usize,
    constraint_dependents_remeasured: usize,
    constraint_dependents_skipped: usize,
    resize_text_relayouts: usize,
    resize_text_reshapes: usize,
    dynamic_contexts_considered: usize,
    dynamic_solver_runs: usize,
    dynamic_fast_fit: usize,
    dynamic_fast_adjust: usize,
    dynamic_break_comparisons: usize,
    dynamic_opportunities_considered: usize,
    dynamic_envelope_queries: usize,
    dynamic_envelope_hits: usize,
    dynamic_envelope_misses: usize,
    dynamic_envelope_rebuilds: usize,
    dynamic_cost_segments_visited: usize,
    dynamic_candidates_created: usize,
    dynamic_candidates_pruned: usize,
    dynamic_deep_expansions: usize,
    dynamic_beam_states: usize,
    dynamic_child_resolves: usize,
    dynamic_child_reflows: usize,
    dynamic_structural_adaptations: usize,
    dynamic_budget_fallbacks: usize,
    dynamic_allocations: usize,
    dynamic_temp_bytes: usize,
    dynamic_solver_reuses: usize,
    dynamic_previous_result_hits: usize,
    dynamic_incremental_assignments: usize,
    dynamic_cold_solves: usize,
    dynamic_pruned_candidates: usize,
    hit_test_nodes_rebuilt: Option<usize>,
    accessibility_projected: usize,
    render_nodes_extracted: usize,
}

#[derive(Serialize)]
struct Stat {
    min: f64,
    p50: f64,
    p95: f64,
    mean: f64,
}

impl Stat {
    fn from_durations(mut samples: Vec<Duration>) -> Self {
        samples.sort_unstable();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let at = |q: f64| ms(samples[((samples.len() as f64 - 1.0) * q).round() as usize]);
        Self {
            min: ms(samples[0]),
            p50: at(0.5),
            p95: at(0.95),
            mean: samples.iter().map(|d| ms(*d)).sum::<f64>() / samples.len() as f64,
        }
    }
}

fn measure(
    operation: Operation,
    rows: usize,
    filler: usize,
    parked: bool,
    append: bool,
    samples: usize,
    warmup: usize,
) -> Cell {
    let mut card = build(rows, filler, parked, append);
    let viewport = LayoutViewport::new(480.0, 100_000.0);
    let shaper = &mut MeasureTextShaper;
    for _ in 0..4 {
        card.runtime.flush(viewport, shaper).unwrap();
    }
    let nodes = card.runtime.context().world().len();
    let mut writes = Vec::with_capacity(samples);
    let mut flushes = Vec::with_capacity(samples);
    let mut stage_samples: Vec<Vec<Duration>> =
        FrameStage::ALL.iter().map(|_| Vec::new()).collect();
    let mut stage_ran = [false; FrameStage::ALL.len()];
    let mut generations = 0u64;
    let mut idle = 0usize;
    let mut measured = 0usize;
    let mut reused = 0usize;
    let mut last = None;
    for iteration in 0..(warmup + samples) {
        let toggled = iteration % 2 == 0;
        let generation = card.runtime.context().world().generation();
        let started = Instant::now();
        match operation {
            Operation::Noop => paint(&mut card),
            Operation::Select => {
                let target = if toggled { rows - 1 } else { rows - 2 };
                let previous = card.selected;
                card.selected = target;
                let context = card.runtime.context_mut();
                for (index, selected) in [(previous, false), (target, true)] {
                    context
                        .update_component(card.rows[index].item, |item, _| item.selected = selected)
                        .unwrap();
                }
            }
            Operation::Label => {
                let label = if toggled {
                    format!("Motion {} (loop)", rows - 1)
                } else {
                    format!("Motion {}", rows - 1)
                };
                card.labels[rows - 1] = label.clone();
                card.runtime
                    .context_mut()
                    .update_component(card.rows[rows - 1].item, |item, _| item.label = label)
                    .unwrap();
            }
        }
        let write = started.elapsed();
        let bumped = card.runtime.context().world().generation() - generation;
        let started = Instant::now();
        let update = card.runtime.flush(viewport, shaper).unwrap();
        let flush = started.elapsed();
        if iteration < warmup {
            continue;
        }
        writes.push(write);
        flushes.push(flush);
        generations += bumped;
        // Counters survive an idle flush unchanged; an idle frame did no
        // layout work, so it adds nothing.
        if update.is_idle() {
            idle += 1;
            continue;
        }
        let work = card.runtime.context().last_work_counters();
        measured += work.layout_children_measured;
        reused += work.layout_placement_plans_reused;
        let profile = card.runtime.context().last_frame_profile();
        for (index, stage) in FrameStage::ALL.into_iter().enumerate() {
            let timing = profile.stage(stage).unwrap();
            stage_samples[index].push(timing.duration);
            stage_ran[index] |= timing.status == StageStatus::Ran;
        }
        last = Some((
            card.runtime.context().last_work_counters(),
            update.accessibility.updated.len(),
        ));
    }
    let stages_p50_ms = FrameStage::ALL
        .into_iter()
        .enumerate()
        .zip(stage_samples)
        .filter(|((index, _), samples)| stage_ran[*index] && !samples.is_empty())
        .map(|((_, stage), samples)| (format!("{stage:?}"), Stat::from_durations(samples).p50))
        .collect();
    let (counters, accessibility_projected) = last.unwrap_or_default();
    Cell {
        operation: operation.name(),
        rows,
        filler,
        parked,
        append,
        nodes,
        write_ms: Stat::from_durations(writes),
        flush_ms: Stat::from_durations(flushes),
        generations: generations as f64 / samples as f64,
        idle_flushes: idle,
        stages_p50_ms,
        children_measured: measured as f64 / samples as f64,
        plans_reused: reused as f64 / samples as f64,
        layout_nodes: counters.layout_nodes,
        layout_frontier_seeds: counters.layout_frontier_seeds,
        layout_frontier_seed_merges: counters.layout_frontier_seed_merges,
        layout_frontier_nodes_measure: counters.layout_frontier_nodes_measure,
        layout_frontier_nodes_placement: counters.layout_frontier_nodes_placement,
        layout_frontier_contexts: counters.layout_frontier_contexts,
        layout_dependency_edges_visited: counters.layout_dependency_edges_visited,
        layout_propagations_stopped: counters.layout_propagations_stopped,
        layout_local_subtree_fallbacks: counters.layout_local_subtree_fallbacks,
        layout_full_document_fallbacks: counters.layout_full_document_fallbacks,
        layout_suffixes_replayed: counters.layout_suffixes_replayed,
        layout_children_measured: counters.layout_children_measured,
        layout_containers_uncacheable: counters.layout_containers_uncacheable,
        layout_result_children_visited: counters.layout_result_children_visited,
        layout_placement_plans_reused: counters.layout_placement_plans_reused,
        layout_measure_plans_reused: counters.layout_measure_plans_reused,
        layout_plan_misses: counters.layout_plan_misses,
        layout_plan_rebuilds: counters.layout_plan_rebuilds,
        layout_scratch_entries: counters.layout_scratch_entries,
        layout_scratch_bytes: counters.layout_scratch_bytes,
        intrinsic_measure_requests: counters.intrinsic_measure_requests,
        intrinsic_measure_full_subtrees: counters.intrinsic_measure_full_subtrees,
        constraint_change_seeds: counters.constraint_change_seeds,
        constraint_dependents_considered: counters.constraint_dependents_considered,
        constraint_dependents_remeasured: counters.constraint_dependents_remeasured,
        constraint_dependents_skipped: counters.constraint_dependents_skipped,
        resize_text_relayouts: counters.resize_text_relayouts,
        resize_text_reshapes: counters.resize_text_reshapes,
        dynamic_contexts_considered: counters.dynamic.contexts_considered,
        dynamic_solver_runs: counters.dynamic.solver_runs,
        dynamic_fast_fit: counters.dynamic.fast_fit,
        dynamic_fast_adjust: counters.dynamic.fast_adjust,
        dynamic_break_comparisons: counters.dynamic.break_comparisons,
        dynamic_opportunities_considered: counters.dynamic.opportunities_considered,
        dynamic_envelope_queries: counters.dynamic.envelope_queries,
        dynamic_envelope_hits: counters.dynamic.envelope_hits,
        dynamic_envelope_misses: counters.dynamic.envelope_misses,
        dynamic_envelope_rebuilds: counters.dynamic.envelope_rebuilds,
        dynamic_cost_segments_visited: counters.dynamic.cost_segments_visited,
        dynamic_candidates_created: counters.dynamic.candidates_created,
        dynamic_candidates_pruned: counters.dynamic.candidates_pruned,
        dynamic_deep_expansions: counters.dynamic.deep_expansions,
        dynamic_beam_states: counters.dynamic.beam_states,
        dynamic_child_resolves: counters.dynamic.child_resolves,
        dynamic_child_reflows: counters.dynamic.child_reflows,
        dynamic_structural_adaptations: counters.dynamic.structural_adaptations,
        dynamic_budget_fallbacks: counters.dynamic.budget_fallbacks,
        dynamic_allocations: counters.dynamic.allocations,
        dynamic_temp_bytes: counters.dynamic.temp_bytes,
        dynamic_solver_reuses: counters.dynamic.solver_reuses,
        dynamic_previous_result_hits: counters.dynamic.previous_result_hits,
        dynamic_incremental_assignments: counters.dynamic.incremental_assignments,
        dynamic_cold_solves: counters.dynamic.cold_solves,
        dynamic_pruned_candidates: counters.dynamic.pruned_candidates,
        hit_test_nodes_rebuilt: counters.hit_test_nodes_rebuilt,
        accessibility_projected,
        render_nodes_extracted: counters.render_nodes_extracted,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let text = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|index| args.get(index + 1))
            .cloned()
    };
    let list = |flag: &str, default: &[usize]| {
        text(flag).map_or_else(
            || default.to_vec(),
            |raw| {
                raw.split(',')
                    .map(|part| part.parse().expect("number"))
                    .collect()
            },
        )
    };
    let samples = text("--samples").map_or(100, |raw| raw.parse().expect("--samples"));
    let warmup = text("--warmup").map_or(20, |raw| raw.parse().expect("--warmup"));
    let operations = text("--op").map_or_else(
        || Operation::ALL.map(|(operation, _)| operation).to_vec(),
        |raw| vec![Operation::parse(&raw).expect("--op noop|select|label")],
    );
    let switch = |flag: &str| {
        text(flag).map(|raw| match raw.as_str() {
            "on" => true,
            "off" => false,
            other => panic!("{flag} on|off, not {other}"),
        })
    };
    let parked = switch("--parked").map_or_else(|| vec![true, false], |parked| vec![parked]);
    let append = switch("--append").unwrap_or(true);
    let rows = list("--rows", &[40, 400]);
    let fillers = list("--filler", &[0, 2000, 8000]);

    let mut cells = Vec::new();
    for operation in operations {
        for parked in parked.iter().copied() {
            for rows in rows.iter().copied() {
                for filler in fillers.iter().copied() {
                    let cell = measure(operation, rows, filler, parked, append, samples, warmup);
                    eprintln!(
                        "{:<6} parked={:<5} rows={:<4} filler={:<5} nodes={:<6} write min={:.4} p50={:.4}  flush min={:.4} p50={:.4}  gen={:.1} idle={} measured={:.1} reused={:.1} layout={} hit={:?} a11y={} render={}",
                        cell.operation,
                        cell.parked,
                        cell.rows,
                        cell.filler,
                        cell.nodes,
                        cell.write_ms.min,
                        cell.write_ms.p50,
                        cell.flush_ms.min,
                        cell.flush_ms.p50,
                        cell.generations,
                        cell.idle_flushes,
                        cell.children_measured,
                        cell.plans_reused,
                        cell.layout_nodes,
                        cell.hit_test_nodes_rebuilt,
                        cell.accessibility_projected,
                        cell.render_nodes_extracted,
                    );
                    for (stage, p50) in &cell.stages_p50_ms {
                        if *p50 > 0.0005 {
                            eprintln!("        {stage:<14} {p50:.4} ms");
                        }
                    }
                    cells.push(cell);
                }
            }
        }
    }
    let report = Report {
        schema_version: 1,
        phase: "card-update",
        samples,
        warmup,
        cells,
    };
    let json = serde_json::to_string_pretty(&report).expect("serialize report") + "\n";
    match text("--output") {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent).expect("report directory");
            }
            fs::write(path, json).expect("write report");
        }
        None => print!("{json}"),
    }
}
