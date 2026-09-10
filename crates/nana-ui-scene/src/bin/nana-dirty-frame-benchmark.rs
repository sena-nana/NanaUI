//! Two-dimensional dirty-frame cost: how one `RuntimeDocument::flush` scales
//! with the number of DIRTY nodes and, independently, with the number of TOTAL
//! nodes in the document.
//!
//! `docs/input-cost.md` established that a Vue frame touching 2 widgets in a
//! 2,000-node document spends 1.51 ms inside this flush, while an idle flush on
//! the same tree is 0.0001 ms. One dirty-set size is not enough to say which
//! system pipeline charges by total size, so this sweeps the grid and reports
//! per-`FrameStage` timings plus the work counters for every cell.
//!
//! Two axes beyond the grid, because conflating either one hides the answer:
//!
//! - `--shape paint|layout|nested|nested-auto|layout-auto`. A bare
//!   background-role swap dirties style and render but schedules no layout; a
//!   height change also schedules layout, and layout invalidation propagates to
//!   ancestors. They scale completely differently. (A Vue hover is NOT the
//!   first one: its CSS cascade emits a whole new `NodeStyle`, which lands in
//!   the second.) The `nested*` shapes edit a LABEL inside a row instead, so
//!   the row's own size cannot move; the `*-auto` shapes make the list
//!   container content-sized, which is what takes it off the definite-size
//!   short circuit in `intrinsic_size_scoped`.
//! - `--position head|tail|spread`. Resizing the FIRST row genuinely shifts
//!   every row below it, so O(total nodes) there is work that is owed and
//!   proves nothing. Resizing the LAST rows shifts nothing, so any cost that
//!   still grows with the document is over-invalidation.
//!
//! `last_frame_profile` / `last_work_counters` retain the last NON-IDLE frame,
//! so reading them after an idle flush silently returns the mount frame. Every
//! sample here asserts the frame did work.

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, SemanticColorRole};
use nana_ui_runtime::{
    AppContext, DocumentId, FrameStage, LayoutViewport, MeasureTextShaper, MutationQueue, NodeKind,
    NodeStyle, StableNodeId, StageStatus, TextContent, WorkCounters,
};
use nana_ui_scene::RuntimeDocument;
use serde::Serialize;

const DOCUMENT: u64 = 1;

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    phase: &'static str,
    profile: &'static str,
    samples: usize,
    warmup: usize,
    cells: Vec<Cell>,
}

#[derive(Serialize)]
struct Cell {
    shape: &'static str,
    position: &'static str,
    rows: usize,
    /// Nodes actually in the document (root + column + row/label pairs).
    nodes: usize,
    /// Rows mutated per frame. Each row is one paint-only style write.
    dirty_rows: usize,
    flush_ms: Stat,
    stages_ms: Vec<StageCell>,
    counters: Counters,
    /// Ground truth from the returned frame update, not from the counters.
    /// `accessibility_nodes_updated` reports the SCHEDULED dirty set
    /// (`schedule.rs`: `self.accessibility.len()`), so it cannot see the
    /// subtree expansion `project_accessibility_delta` performs internally.
    projected: Projected,
    layout_substages_ms: LayoutSubstages,
}

#[derive(Serialize)]
struct Projected {
    accessibility_updated: usize,
    accessibility_removed: usize,
    scene_updated_nodes: usize,
    scene_rebuilt_primitives: usize,
    passes: usize,
}

/// Attribution inside the single `FrameStage::Layout` number.
#[derive(Serialize)]
struct LayoutSubstages {
    tooltips_ms: f64,
    engine_ms: f64,
    writeback_commit_ms: f64,
    scroll_metrics_ms: f64,
}

#[derive(Serialize)]
struct StageCell {
    stage: String,
    status: &'static str,
    p50_ms: f64,
    mean_ms: f64,
}

#[derive(Serialize)]
struct Counters {
    entities_total: usize,
    entities_changed: usize,
    style_processed: usize,
    text_shaped: usize,
    layout_nodes: usize,
    hit_test_candidates: usize,
    accessibility_nodes_updated: usize,
    render_nodes_changed: usize,
    render_nodes_extracted: usize,
}

impl From<WorkCounters> for Counters {
    fn from(value: WorkCounters) -> Self {
        Self {
            entities_total: value.entities_total,
            entities_changed: value.entities_changed,
            style_processed: value.style_processed,
            text_shaped: value.text_shaped,
            layout_nodes: value.layout_nodes,
            hit_test_candidates: value.hit_test_candidates,
            accessibility_nodes_updated: value.accessibility_nodes_updated,
            render_nodes_changed: value.render_nodes_changed,
            render_nodes_extracted: value.render_nodes_extracted,
        }
    }
}

#[derive(Serialize)]
struct Stat {
    p50: f64,
    p95: f64,
    mean: f64,
    max: f64,
}

impl Stat {
    fn from_durations(mut samples: Vec<Duration>) -> Self {
        samples.sort_unstable();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let at = |q: f64| {
            let index = ((samples.len() as f64 - 1.0) * q).round() as usize;
            ms(samples[index])
        };
        let mean = samples.iter().map(|d| ms(*d)).sum::<f64>() / samples.len() as f64;
        Self {
            p50: at(0.5),
            p95: at(0.95),
            mean,
            max: ms(*samples.last().unwrap()),
        }
    }
}

fn id(raw: u64) -> StableNodeId {
    StableNodeId::new(raw).unwrap()
}

fn row_id(row: usize) -> StableNodeId {
    id(3 + row as u64 * 2)
}

fn label_id(row: usize) -> StableNodeId {
    id(4 + row as u64 * 2)
}

fn row_layout() -> Arc<LayoutStyle> {
    Arc::new(LayoutStyle {
        width: Some(LengthSpec::Px(300.0)),
        height: Some(LengthSpec::Px(20.0)),
        direction: Some(FlexDirection::Row),
        ..LayoutStyle::default()
    })
}

/// A flat column of `rows` rows, each with one text label. 2 * rows + 2 nodes.
fn build(shape: Shape, rows: usize) -> RuntimeDocument {
    let document = DocumentId::new(DOCUMENT).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let mut queue = MutationQueue::new();
    queue.create(id(1), document, NodeKind::Document);
    queue.create(id(2), document, NodeKind::Element { tag: "div".into() });
    queue.insert(id(1), id(2), None);
    queue.set_style(
        id(2),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(300.0)),
                // Content-sized on the main axis for the `*-auto` shapes: the
                // container's own height then depends on its children.
                height: (!shape.container_hugs()).then_some(LengthSpec::Fill),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    for row in 0..rows {
        let row_node = row_id(row);
        let label = id(4 + row as u64 * 2);
        queue.create(row_node, document, NodeKind::Element { tag: "div".into() });
        queue.create(label, document, NodeKind::Text);
        queue.insert(id(2), row_node, None);
        queue.insert(row_node, label, None);
        queue.set_text(
            label,
            TextContent {
                value: format!("row {row}"),
            },
        );
        queue.set_style(
            row_node,
            NodeStyle {
                layout: row_layout(),
                ..NodeStyle::default()
            },
        );
        queue.set_style(label, NodeStyle::default());
    }
    runtime.context_mut().commit_mutations(queue).unwrap();
    runtime
}

/// What the per-frame mutation touches.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Swap a background role: dirties style + render, schedules no layout.
    /// This is what a hover highlight is at the Runtime boundary.
    Paint,
    /// Change a row's height: dirties style AND schedules layout, which
    /// propagates to ancestors and therefore reaches the document root.
    Layout,
    /// Change a LABEL inside a fixed-size row: layout-dirty, propagating to the
    /// root like `Layout`, but the row's own size cannot move, so no sibling
    /// row can shift. The list container is dirty and must still not pay for
    /// its other children.
    Nested,
    /// `Nested`, but the container is CONTENT-SIZED. Its own height depends on
    /// its children, so the definite-size short circuit in
    /// `intrinsic_size_scoped` cannot fire. `MeasurePlan` is what keeps this
    /// constant: every child hits the retained intrinsic memo and is unchanged,
    /// so the container reuses its own cached measurement.
    NestedAuto,
    /// `Layout` under a CONTENT-SIZED container: the row really does change
    /// size, so the container's own height really does move and its measure
    /// plan is correctly rejected. What it costs then is a full re-measure of
    /// every child -- the measure-side analogue of the placement suffix replay,
    /// which does not exist. `tail` is the interesting column: the resized row
    /// is last, so nothing below it moves and the only work owed is the
    /// container's new total.
    LayoutAuto,
}

impl Shape {
    /// Whether the list container is sized by its children.
    fn container_hugs(self) -> bool {
        matches!(self, Self::NestedAuto | Self::LayoutAuto)
    }
}

/// Where in the document the dirty rows sit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Head,
    Tail,
    Spread,
}

impl Position {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "head" => Some(Self::Head),
            "tail" => Some(Self::Tail),
            "spread" => Some(Self::Spread),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Tail => "tail",
            Self::Spread => "spread",
        }
    }
}

impl Shape {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "paint" => Some(Self::Paint),
            "layout" => Some(Self::Layout),
            "nested" => Some(Self::Nested),
            "nested-auto" => Some(Self::NestedAuto),
            "layout-auto" => Some(Self::LayoutAuto),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Paint => "paint",
            Self::Layout => "layout",
            Self::Nested => "nested",
            Self::NestedAuto => "nested-auto",
            Self::LayoutAuto => "layout-auto",
        }
    }
}

fn dirty(context: &mut AppContext, shape: Shape, targets: &[usize], toggled: bool) {
    let mut queue = MutationQueue::new();
    for row in targets {
        let style = match shape {
            Shape::Paint => NodeStyle {
                layout: row_layout(),
                background: toggled.then_some(SemanticColorRole::Hover),
                ..NodeStyle::default()
            },
            Shape::Layout | Shape::LayoutAuto => NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(300.0)),
                    height: Some(LengthSpec::Px(if toggled { 22.0 } else { 20.0 })),
                    direction: Some(FlexDirection::Row),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
            Shape::Nested | Shape::NestedAuto => NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(if toggled { 90.0 } else { 80.0 })),
                    height: Some(LengthSpec::Px(12.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        };
        let target = match shape {
            Shape::Nested | Shape::NestedAuto => label_id(*row),
            _ => row_id(*row),
        };
        queue.set_style(target, style);
    }
    context.commit_mutations(queue).unwrap();
}

fn status_name(status: StageStatus) -> &'static str {
    match status {
        StageStatus::Ran => "ran",
        StageStatus::Skipped => "skipped",
        StageStatus::Unsupported => "unsupported",
    }
}

fn measure(
    shape: Shape,
    position: Position,
    rows: usize,
    dirty_rows: usize,
    samples: usize,
    warmup: usize,
) -> Cell {
    let mut runtime = build(shape, rows);
    let viewport = LayoutViewport::new(300.0, 800.0);
    let mut shaper = MeasureTextShaper;
    // Mount. This is the one full frame; everything after it is incremental.
    let mount = runtime.flush(viewport, &mut shaper).unwrap();
    assert!(!mount.is_idle(), "mount frame must do work");
    // Settle any follow-up passes so the measured frames start from idle.
    for _ in 0..4 {
        runtime.flush(viewport, &mut shaper).unwrap();
    }

    // WHERE the dirty rows sit decides how much reflow is genuinely owed.
    // Resizing the first row really does shift every row below it, so O(total)
    // there is correct and says nothing. Resizing the LAST rows shifts nothing:
    // any cost that still scales with the document is over-invalidation.
    let targets: Vec<usize> = match position {
        Position::Tail => (0..dirty_rows).map(|i| rows - 1 - i).collect(),
        Position::Head => (0..dirty_rows).collect(),
        Position::Spread => {
            let stride = (rows / dirty_rows.max(1)).max(1);
            (0..dirty_rows).map(|i| (i * stride) % rows).collect()
        }
    };

    let mut flushes = Vec::with_capacity(samples);
    let mut stage_totals = [Duration::ZERO; 13];
    let mut stage_samples: Vec<Vec<Duration>> =
        (0..13).map(|_| Vec::with_capacity(samples)).collect();
    let mut stage_status = [StageStatus::Skipped; 13];
    let mut counters = WorkCounters::default();
    let mut substage_totals = [Duration::ZERO; 4];
    let mut projected = Projected {
        accessibility_updated: 0,
        accessibility_removed: 0,
        scene_updated_nodes: 0,
        scene_rebuilt_primitives: 0,
        passes: 0,
    };

    for iteration in 0..(warmup + samples) {
        let hovered = iteration % 2 == 0;
        dirty(runtime.context_mut(), shape, &targets, hovered);
        let _ = runtime.context_mut().take_layout_substage_totals();
        let started = Instant::now();
        let update = runtime.flush(viewport, &mut shaper).unwrap();
        let elapsed = started.elapsed();
        // The profile/counter accessors retain the last NON-IDLE frame, so a
        // sample that did no work would silently report the mount frame.
        assert!(
            !update.is_idle(),
            "dirty frame reported idle at rows={rows} dirty={dirty_rows}"
        );
        let substages = runtime.context_mut().take_layout_substage_totals();
        if iteration < warmup {
            continue;
        }
        for (total, elapsed) in substage_totals.iter_mut().zip(substages) {
            *total += elapsed;
        }
        flushes.push(elapsed);
        projected = Projected {
            accessibility_updated: update.accessibility.updated.len(),
            accessibility_removed: update.accessibility.removed.len(),
            scene_updated_nodes: update.scene.updated_nodes,
            scene_rebuilt_primitives: update.scene.rebuilt_primitives,
            passes: update.passes,
        };
        let profile = runtime.context().last_frame_profile();
        for (index, stage) in FrameStage::ALL.into_iter().enumerate() {
            let timing = profile.stage(stage).unwrap();
            stage_totals[index] += timing.duration;
            stage_samples[index].push(timing.duration);
            stage_status[index] = timing.status;
        }
        counters = runtime.context().last_work_counters();
    }

    let stages_ms = FrameStage::ALL
        .into_iter()
        .enumerate()
        .map(|(index, stage)| {
            let mut sorted = std::mem::take(&mut stage_samples[index]);
            sorted.sort_unstable();
            let p50 = sorted[(sorted.len() - 1) / 2].as_secs_f64() * 1000.0;
            StageCell {
                stage: format!("{stage:?}"),
                status: status_name(stage_status[index]),
                p50_ms: p50,
                mean_ms: stage_totals[index].as_secs_f64() * 1000.0 / samples as f64,
            }
        })
        .collect();

    Cell {
        shape: shape.name(),
        position: position.name(),
        rows,
        nodes: rows * 2 + 2,
        dirty_rows,
        flush_ms: Stat::from_durations(flushes),
        stages_ms,
        counters: counters.into(),
        projected,
        layout_substages_ms: LayoutSubstages {
            tooltips_ms: ms_mean(substage_totals[0], samples),
            engine_ms: ms_mean(substage_totals[1], samples),
            writeback_commit_ms: ms_mean(substage_totals[2], samples),
            scroll_metrics_ms: ms_mean(substage_totals[3], samples),
        },
    }
}

fn ms_mean(total: Duration, samples: usize) -> f64 {
    total.as_secs_f64() * 1000.0 / samples as f64
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|index| args.get(index + 1))
            .and_then(|raw| raw.parse::<usize>().ok())
    };
    let samples = value("--samples").unwrap_or(200);
    let warmup = value("--warmup").unwrap_or(40);
    let output = args
        .iter()
        .position(|arg| arg == "--output")
        .and_then(|index| args.get(index + 1))
        .cloned();

    let positions: Vec<Position> = args
        .iter()
        .position(|arg| arg == "--position")
        .and_then(|index| args.get(index + 1))
        .and_then(|raw| Position::parse(raw))
        .map_or_else(
            || vec![Position::Tail, Position::Head],
            |position| vec![position],
        );
    let shapes: Vec<Shape> = args
        .iter()
        .position(|arg| arg == "--shape")
        .and_then(|index| args.get(index + 1))
        .and_then(|raw| Shape::parse(raw))
        .map_or_else(
            || {
                vec![
                    Shape::Paint,
                    Shape::Layout,
                    Shape::Nested,
                    Shape::NestedAuto,
                    Shape::LayoutAuto,
                ]
            },
            |shape| vec![shape],
        );

    let row_counts =
        value("--rows").map_or_else(|| vec![250usize, 500, 1000, 2000, 4000], |rows| vec![rows]);
    let dirty_counts =
        value("--dirty").map_or_else(|| vec![1usize, 2, 8, 32, 128], |dirty| vec![dirty]);

    let mut cells = Vec::new();
    for shape in shapes {
        for position in positions.iter().copied() {
            for rows in row_counts.iter().copied() {
                for dirty_rows in dirty_counts.iter().copied() {
                    if dirty_rows > rows {
                        continue;
                    }
                    let cell = measure(shape, position, rows, dirty_rows, samples, warmup);
                    eprintln!(
                        "{:<7} {:<6} rows={:<5} nodes={:<5} dirty={:<4} flush p50={:.4} ms  style={} layout={} hit={} a11y={} render={}",
                        cell.shape,
                        cell.position,
                        cell.rows,
                        cell.nodes,
                        cell.dirty_rows,
                        cell.flush_ms.p50,
                        cell.counters.style_processed,
                        cell.counters.layout_nodes,
                        cell.counters.hit_test_candidates,
                        cell.counters.accessibility_nodes_updated,
                        cell.counters.render_nodes_changed,
                    );
                    eprintln!(
                        "        projected: a11y={} scene_nodes={} prims={} passes={}",
                        cell.projected.accessibility_updated,
                        cell.projected.scene_updated_nodes,
                        cell.projected.scene_rebuilt_primitives,
                        cell.projected.passes,
                    );
                    eprintln!(
                        "        layout substages mean: tooltips={:.4} engine={:.4} writeback={:.4} scroll_metrics={:.4}",
                        cell.layout_substages_ms.tooltips_ms,
                        cell.layout_substages_ms.engine_ms,
                        cell.layout_substages_ms.writeback_commit_ms,
                        cell.layout_substages_ms.scroll_metrics_ms,
                    );
                    for stage in &cell.stages_ms {
                        if stage.status == "ran" && stage.p50_ms > 0.0 {
                            eprintln!("      {:<14} {:.4} ms", stage.stage, stage.p50_ms);
                        }
                    }
                    cells.push(cell);
                }
            }
        }
    }

    let report = Report {
        schema_version: 1,
        phase: "dirty-frame-scaling",
        profile: "release",
        samples,
        warmup,
        cells,
    };
    let json = serde_json::to_string_pretty(&report).expect("serialize report") + "\n";
    if let Some(path) = output {
        let path = std::path::PathBuf::from(path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).expect("report directory");
        }
        fs::write(path, json).expect("write report");
    } else {
        print!("{json}");
    }
}
