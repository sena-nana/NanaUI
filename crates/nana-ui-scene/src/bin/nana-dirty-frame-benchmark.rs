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
//! - `--shape paint|layout|nested|nested-auto|layout-auto|layout-rtl|layout-reverse`.
//!   A bare
//!   background-role swap dirties style and render but schedules no layout; a
//!   height change also schedules layout, and layout invalidation propagates to
//!   ancestors. They scale completely differently. (A Vue hover is NOT the
//!   first one: its CSS cascade emits a whole new `NodeStyle`, which lands in
//!   the second.) The `nested*` shapes edit a LABEL inside a row instead, so
//!   the row's own size cannot move; the `*-auto` shapes make the list
//!   container content-sized, which is what takes it off the definite-size
//!   short circuit in `intrinsic_size_scoped`. `layout-rtl` is `layout` in an
//!   RTL list (its cross axis — the inline one — runs from the right) and
//!   `layout-reverse` stacks the list bottom-up (`column-reverse`): the two
//!   reversed axes a container's sequential replay has to express.
//! - `--position head|tail|spread`. Resizing the FIRST row genuinely shifts
//!   every row below it, so O(total nodes) there is work that is owed and
//!   proves nothing. Resizing the LAST rows shifts nothing, so any cost that
//!   still grows with the document is over-invalidation.
//!
//! - `--engine measure|nana-text`. `measure` (the default) is the em-width
//!   test shaper the #33 numbers were recorded with. `nana-text` resolves the
//!   labels through a real `nana-text` engine holding the bundled UI face, so
//!   every label retains a laid-out `TextLayout` (Issue #95).
//!
//! `last_frame_profile` / `last_work_counters` retain the last NON-IDLE frame,
//! so reading them after an idle flush silently returns the mount frame. Every
//! sample here asserts the frame did work.

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_text::TextWorkCounters;
use nana_ui_core::{DirSpec, FlexDirection, LayoutStyle, LengthSpec, SemanticColorRole};
use nana_ui_runtime::{
    AppContext, DocumentId, FrameStage, LayoutViewport, MeasureTextShaper, MutationQueue,
    NanaTextEngineShaper, NodeKind, NodeStyle, StableNodeId, StageStatus, TextContent, TextShaper,
    WorkCounters, text_shape_stats,
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
    engine: &'static str,
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
    layout_plan: LayoutPlan,
    text_shape: TextShapePass,
    text_work: TextWork,
}

/// Issue #95 text work per frame (mean over samples, fractional so work done
/// on some samples only is not rounded away): how many candidates the
/// text passes looked at, how many were decided on revision alone, and what
/// the rest cost. With `text_nodes_shaped == 0` these explain what TextShape
/// still spends.
#[derive(Serialize)]
struct TextWork {
    text_nodes_considered: f64,
    text_nodes_revision_skipped: f64,
    text_nodes_shaped: f64,
    text_source_clones: f64,
    text_bytes_hashed: f64,
    shape_cache_lookup: f64,
    layout_cache_lookup: f64,
    layouts_created: f64,
    constraint_only_relayouts: f64,
    text_layouts_reused: f64,
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
/// What the layout engine's retained container plans saved, per frame (mean
/// over samples). `children_measured` is the sibling scan a plan exists to
/// avoid: constant when the plan held, O(rows) when the container relaid out.
#[derive(Serialize)]
struct LayoutPlan {
    children_measured: f64,
    plans_reused: f64,
    suffixes_replayed: f64,
}

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
    text_shaped_runs: usize,
    text_layout_cache_hits: usize,
    text_layout_cache_misses: usize,
    text_wrap_layouts: usize,
    cache_eviction: Option<usize>,
    allocations: usize,
    allocated_bytes: usize,
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
            text_shaped_runs: value.text_shaped_runs,
            text_layout_cache_hits: value.text_layout_cache_hits,
            text_layout_cache_misses: value.text_layout_cache_misses,
            text_wrap_layouts: value.text_wrap_layouts,
            cache_eviction: value.cache_eviction,
            allocations: value.allocations,
            allocated_bytes: value.allocated_bytes,
            layout_nodes: value.layout_nodes,
            hit_test_candidates: value.hit_test_candidates,
            accessibility_nodes_updated: value.accessibility_nodes_updated,
            render_nodes_changed: value.render_nodes_changed,
            render_nodes_extracted: value.render_nodes_extracted,
        }
    }
}

/// Attribution inside the single `FrameStage::TextShape` number. Not `WorkCounters`.
/// Instruments the host-shaper path only; under `--engine nana-text` engine
/// copies, hashing and lookups are reported in `text_work` instead.
#[derive(Serialize)]
struct TextShapePass {
    scope_nodes: usize,
    nonempty_text_nodes: usize,
    string_clones: usize,
    string_clone_bytes: usize,
    key_builds: usize,
    cache_lookups: usize,
    skipped_unchanged: usize,
    clone_ms: f64,
    key_ms: f64,
    lookup_ms: f64,
    inner_shape_ms: f64,
}

#[derive(Serialize)]
struct Stat {
    /// The least disturbed sample: what to compare A/B on a loaded machine.
    min: f64,
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
            min: ms(samples[0]),
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

/// The row counts swept when `--rows` is not given.
///
/// This is a contract, not a convenience: `--shape layout --position head` at
/// 1000 / 2000 / 4000 rows is the Issue #33 workload, and Issue #89 keeps it as
/// the text-migration baseline that every `nana-text` phase is re-measured
/// against. Narrowing the grid would silently retire that baseline, so
/// `the_migration_grid_still_spans_two_four_and_eight_thousand_nodes` asserts
/// the three cells are still here.
const DIRTY_FRAME_ROW_GRID: [usize; 5] = [250, 500, 1000, 2000, 4000];

/// Nodes in a document of `rows` rows. See [`build`].
const fn document_nodes(rows: usize) -> usize {
    2 * rows + 2
}

/// A flat column of `rows` rows, each with one text label. See
/// [`document_nodes`] for the node count.
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
                flex_reverse: shape == Shape::LayoutReverse,
                dir: (shape == Shape::LayoutRtl).then_some(DirSpec::Rtl),
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
    /// `Layout` in an RTL list: the container is `direction: rtl`, so its
    /// cross axis — the inline one — starts at the right edge. The rows fill
    /// the width, so nothing moves on screen; what it measures is that a
    /// reversed axis keeps the placement plan's sequential replay.
    LayoutRtl,
    /// `Layout` in a `column-reverse` list: the main axis runs bottom-up, so
    /// resizing the first row moves every row *above* it and the tail rows
    /// sit at the top.
    LayoutReverse,
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
            "layout-rtl" => Some(Self::LayoutRtl),
            "layout-reverse" => Some(Self::LayoutReverse),
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
            Self::LayoutRtl => "layout-rtl",
            Self::LayoutReverse => "layout-reverse",
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
            Shape::Layout | Shape::LayoutAuto | Shape::LayoutRtl | Shape::LayoutReverse => {
                NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(300.0)),
                        height: Some(LengthSpec::Px(if toggled { 22.0 } else { 20.0 })),
                        direction: Some(FlexDirection::Row),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                }
            }
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

/// Which text backend the frames resolve labels through.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Engine {
    Measure,
    NanaText,
}

impl Engine {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "measure" => Some(Self::Measure),
            "nana-text" => Some(Self::NanaText),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Measure => "measure",
            Self::NanaText => "nana-text",
        }
    }
}

/// An engine holding only the bundled UI face, as the generic `sans-serif`.
fn nana_text_shaper() -> NanaTextEngineShaper {
    use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_bytes(
            font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
            &FaceDescriptor::default(),
        )
        .expect("the bundled UI face registers");
    NanaTextEngineShaper::new(Arc::new(std::sync::Mutex::new(
        nana_text::NativeTextEngine::new(fonts),
    )))
}

fn measure(
    engine: Engine,
    shape: Shape,
    position: Position,
    rows: usize,
    dirty_rows: usize,
    samples: usize,
    warmup: usize,
) -> Cell {
    match engine {
        Engine::Measure => measure_with(
            engine,
            &mut MeasureTextShaper,
            shape,
            position,
            rows,
            dirty_rows,
            samples,
            warmup,
        ),
        Engine::NanaText => measure_with(
            engine,
            &mut nana_text_shaper(),
            shape,
            position,
            rows,
            dirty_rows,
            samples,
            warmup,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn measure_with(
    engine: Engine,
    shaper: &mut impl TextShaper,
    shape: Shape,
    position: Position,
    rows: usize,
    dirty_rows: usize,
    samples: usize,
    warmup: usize,
) -> Cell {
    let mut runtime = build(shape, rows);
    let viewport = LayoutViewport::new(300.0, 800.0);
    // Mount. This is the one full frame; everything after it is incremental.
    let mount = runtime.flush(viewport, shaper).unwrap();
    assert!(!mount.is_idle(), "mount frame must do work");
    // Settle any follow-up passes so the measured frames start from idle.
    for _ in 0..4 {
        runtime.flush(viewport, shaper).unwrap();
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
    let mut plan_totals = [0usize; 3];
    let mut substage_totals = [Duration::ZERO; 4];
    let mut text_shape_totals = text_shape_stats::TextShapePassStats::default();
    let mut text_work_totals = TextWorkCounters::default();
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
        text_shape_stats::reset();
        nana_ui_runtime::plan_stats::reset();
        let started = Instant::now();
        let update = runtime.flush(viewport, shaper).unwrap();
        let elapsed = started.elapsed();
        // The profile/counter accessors retain the last NON-IDLE frame, so a
        // sample that did no work would silently report the mount frame.
        assert!(
            !update.is_idle(),
            "dirty frame reported idle at rows={rows} dirty={dirty_rows}"
        );
        let substages = runtime.context_mut().take_layout_substage_totals();
        let plan = [
            nana_ui_runtime::plan_stats::children_measured(),
            nana_ui_runtime::plan_stats::plans_reused(),
            nana_ui_runtime::plan_stats::suffixes_replayed(),
        ];
        if iteration < warmup {
            continue;
        }
        for (total, count) in plan_totals.iter_mut().zip(plan) {
            *total += count;
        }
        for (total, elapsed) in substage_totals.iter_mut().zip(substages) {
            *total += elapsed;
        }
        text_work_totals.accumulate(runtime.context().world().last_text_work_counters());
        let text_shape = text_shape_stats::snapshot();
        text_shape_totals.scope_nodes = text_shape_totals
            .scope_nodes
            .saturating_add(text_shape.scope_nodes);
        text_shape_totals.nonempty_text_nodes = text_shape_totals
            .nonempty_text_nodes
            .saturating_add(text_shape.nonempty_text_nodes);
        text_shape_totals.string_clones = text_shape_totals
            .string_clones
            .saturating_add(text_shape.string_clones);
        text_shape_totals.string_clone_bytes = text_shape_totals
            .string_clone_bytes
            .saturating_add(text_shape.string_clone_bytes);
        text_shape_totals.key_builds = text_shape_totals
            .key_builds
            .saturating_add(text_shape.key_builds);
        text_shape_totals.cache_lookups = text_shape_totals
            .cache_lookups
            .saturating_add(text_shape.cache_lookups);
        text_shape_totals.skipped_unchanged = text_shape_totals
            .skipped_unchanged
            .saturating_add(text_shape.skipped_unchanged);
        text_shape_totals.clone_ns = text_shape_totals
            .clone_ns
            .saturating_add(text_shape.clone_ns);
        text_shape_totals.key_ns = text_shape_totals.key_ns.saturating_add(text_shape.key_ns);
        text_shape_totals.lookup_ns = text_shape_totals
            .lookup_ns
            .saturating_add(text_shape.lookup_ns);
        text_shape_totals.inner_shape_ns = text_shape_totals
            .inner_shape_ns
            .saturating_add(text_shape.inner_shape_ns);
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
        engine: engine.name(),
        shape: shape.name(),
        position: position.name(),
        rows,
        nodes: document_nodes(rows),
        dirty_rows,
        flush_ms: Stat::from_durations(flushes),
        stages_ms,
        counters: counters.into(),
        projected,
        layout_plan: LayoutPlan {
            children_measured: plan_totals[0] as f64 / samples as f64,
            plans_reused: plan_totals[1] as f64 / samples as f64,
            suffixes_replayed: plan_totals[2] as f64 / samples as f64,
        },
        layout_substages_ms: LayoutSubstages {
            tooltips_ms: ms_mean(substage_totals[0], samples),
            engine_ms: ms_mean(substage_totals[1], samples),
            writeback_commit_ms: ms_mean(substage_totals[2], samples),
            scroll_metrics_ms: ms_mean(substage_totals[3], samples),
        },
        text_shape: TextShapePass {
            scope_nodes: mean_count(text_shape_totals.scope_nodes, samples),
            nonempty_text_nodes: mean_count(text_shape_totals.nonempty_text_nodes, samples),
            string_clones: mean_count(text_shape_totals.string_clones, samples),
            string_clone_bytes: mean_count(text_shape_totals.string_clone_bytes, samples),
            key_builds: mean_count(text_shape_totals.key_builds, samples),
            cache_lookups: mean_count(text_shape_totals.cache_lookups, samples),
            skipped_unchanged: mean_count(text_shape_totals.skipped_unchanged, samples),
            clone_ms: ns_mean_ms(text_shape_totals.clone_ns, samples),
            key_ms: ns_mean_ms(text_shape_totals.key_ns, samples),
            lookup_ms: ns_mean_ms(text_shape_totals.lookup_ns, samples),
            inner_shape_ms: ns_mean_ms(text_shape_totals.inner_shape_ns, samples),
        },
        text_work: TextWork {
            text_nodes_considered: mean(text_work_totals.text_nodes_considered, samples),
            text_nodes_revision_skipped: mean(
                text_work_totals.text_nodes_revision_skipped,
                samples,
            ),
            text_nodes_shaped: mean(text_work_totals.text_nodes_shaped, samples),
            text_source_clones: mean(text_work_totals.text_source_clones, samples),
            text_bytes_hashed: mean(text_work_totals.text_bytes_hashed, samples),
            shape_cache_lookup: mean(text_work_totals.shape_cache_lookups, samples),
            layout_cache_lookup: mean(text_work_totals.layout_cache_lookups, samples),
            layouts_created: mean(text_work_totals.layouts_created, samples),
            constraint_only_relayouts: mean(text_work_totals.constraint_only_relayouts, samples),
            text_layouts_reused: mean(text_work_totals.text_layouts_reused, samples),
        },
    }
}

fn mean(total: usize, samples: usize) -> f64 {
    if samples == 0 {
        0.0
    } else {
        total as f64 / samples as f64
    }
}

fn mean_count(total: usize, samples: usize) -> usize {
    total.checked_div(samples).unwrap_or(0)
}

fn ns_mean_ms(total_ns: u64, samples: usize) -> f64 {
    if samples == 0 {
        0.0
    } else {
        (total_ns as f64 / samples as f64) / 1_000_000.0
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
                    Shape::LayoutRtl,
                    Shape::LayoutReverse,
                ]
            },
            |shape| vec![shape],
        );

    let engine = match args
        .iter()
        .position(|arg| arg == "--engine")
        .and_then(|index| args.get(index + 1))
    {
        None => Engine::Measure,
        Some(raw) => Engine::parse(raw).unwrap_or_else(|| {
            eprintln!("--engine must be `measure` or `nana-text`, not `{raw}`");
            std::process::exit(2)
        }),
    };

    let row_counts =
        value("--rows").map_or_else(|| DIRTY_FRAME_ROW_GRID.to_vec(), |rows| vec![rows]);
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
                    let cell = measure(engine, shape, position, rows, dirty_rows, samples, warmup);
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
                    eprintln!(
                        "        text: shaped={} runs={} cache hit/miss/evict={}/{}/{:?} wrap={} allocs={} bytes={}",
                        cell.counters.text_shaped,
                        cell.counters.text_shaped_runs,
                        cell.counters.text_layout_cache_hits,
                        cell.counters.text_layout_cache_misses,
                        cell.counters.cache_eviction,
                        cell.counters.text_wrap_layouts,
                        cell.counters.allocations,
                        cell.counters.allocated_bytes,
                    );
                    eprintln!(
                        "        text shape pass: scope={} nonempty={} clones={} clone_bytes={} keys={} lookups={} skipped={}",
                        cell.text_shape.scope_nodes,
                        cell.text_shape.nonempty_text_nodes,
                        cell.text_shape.string_clones,
                        cell.text_shape.string_clone_bytes,
                        cell.text_shape.key_builds,
                        cell.text_shape.cache_lookups,
                        cell.text_shape.skipped_unchanged,
                    );
                    eprintln!(
                        "        text work ({}): considered={:.2} revision_skipped={:.2} shaped={:.2} source_clones={:.2} bytes_hashed={:.2} shape_lookups={:.2} layout_lookups={:.2} layouts_created={:.2} reused={:.2}",
                        cell.engine,
                        cell.text_work.text_nodes_considered,
                        cell.text_work.text_nodes_revision_skipped,
                        cell.text_work.text_nodes_shaped,
                        cell.text_work.text_source_clones,
                        cell.text_work.text_bytes_hashed,
                        cell.text_work.shape_cache_lookup,
                        cell.text_work.layout_cache_lookup,
                        cell.text_work.layouts_created,
                        cell.text_work.text_layouts_reused,
                    );
                    eprintln!(
                        "        text shape substages mean: clone={:.4} key={:.4} lookup={:.4} inner={:.4}",
                        cell.text_shape.clone_ms,
                        cell.text_shape.key_ms,
                        cell.text_shape.lookup_ms,
                        cell.text_shape.inner_shape_ms,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #33's finding, and Issue #89's migration baseline, both live in
    /// three cells of this sweep. They are only comparable across phases if the
    /// sweep still contains them.
    #[test]
    fn the_migration_grid_still_spans_two_four_and_eight_thousand_nodes() {
        let nodes: Vec<usize> = DIRTY_FRAME_ROW_GRID
            .iter()
            .copied()
            .map(document_nodes)
            .collect();
        for expected in [2002, 4002, 8002] {
            assert!(
                nodes.contains(&expected),
                "the Issue #33 / #89 migration benchmark must keep the {expected}-node \
                 head-layout-dirty cell; the grid now yields {nodes:?}"
            );
        }
    }

    /// The baseline is quoted in docs as `--shape layout --position head`, so
    /// those two spellings are part of the contract too.
    #[test]
    fn the_migration_grid_is_reachable_by_the_documented_flags() {
        assert!(Shape::parse("layout").is_some());
        assert!(Position::parse("head").is_some());
    }
}
