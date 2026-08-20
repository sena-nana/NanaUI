//! Headless GPUI runner for NanaUI Issue #12 shared Scenario JSON.
//!
//! Excluded from the Nana workspace; not a product renderer and not an #8 gate.
//! Uses crates.io `gpui` 0.2.2 `TestAppContext` (real GPUI element tree, Taffy
//! layout, and scene paint). `TestWindow::draw` does not GPU-present, so this
//! binary omits `present_ms` rather than emitting 0. TestPlatform also does not
//! deliver `on_request_frame`, so `frames_after_idle` is omitted rather than
//! stuffed. Fake timings are not emitted.
//!
//! StaticTree / Mutation / Hover share Nana's complete-binary-heap
//! (`parent(i)=i/2`, root `1`, element-div). VirtualList materializes only the
//! catalog window. Table materializes only the catalog table window. StaticTree
//! 50k is refused. Animation / Ime / Dock / Overlay / TextEditor / GpuScene /
//! VirtualTree stay unsupported.

mod tree;

use gpui::{
    AnyElement, AvailableSpace, EmptyView, TestAppContext, VisualTestContext, point, px, size,
};
use serde::Deserialize;
use serde_json::{Value, json};

use std::cell::Cell;
use std::fs;
use std::ops::{Deref, Range};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use crate::tree::{
    MutationKind, MutationSpec, TABLE_COLUMN_EXTENT_PX, TABLE_ROW_EXTENT_PX, TABLE_SHORT_CELL_LEN,
    TABLE_WRAPPED_CELL_LEN, TABLE_WRAPPED_CELLS, hover_tree, live_ui_entities_bound,
    mutation_target, mutation_tree, static_tree, table_axis_window, table_live_ui_entities_bound,
    tree_provenance, virtual_list_view, virtual_table_view, virtual_window,
};

const VIEWPORT_WIDTH: u32 = 900;
const VIEWPORT_HEIGHT: u32 = 640;
const DEFAULT_WARMUP: usize = 5;
const DEFAULT_ITERATIONS: usize = 20;
const REQUIRED_HOVER_NODES: usize = 10_000;
const REQUIRED_MUTATION_NODES: usize = 5_000;
const INCOMPARABLE_STATIC_TREE_NODES: usize = 50_000;

const EXIT_OK: u8 = 0;
const EXIT_ERROR: u8 = 1;
const EXIT_UNSUPPORTED: u8 = 2;

const ANIMATION_UNSUPPORTED: &str = "unsupported: Animation (no Runtime scheduler)";
const IME_UNSUPPORTED: &str = "unsupported: Ime (no set_ime_preedit/commit_ime)";
const OVERLAY_UNSUPPORTED: &str = "unsupported: Overlay (no OverlayHost activate/dismiss)";
const GPU_SCENE_UNSUPPORTED: &str = "unsupported: GpuScene (no nana-gpu-scene-benchmark path)";
const DOCK_UNSUPPORTED: &str = "unsupported: DockWorkspace (not Nana assemble_dock chrome)";
const TEXT_EDITOR_UNSUPPORTED: &str =
    "unsupported: TextEditor (no replace_text_area_selection+drain_text)";
const VIRTUAL_TREE_UNSUPPORTED: &str =
    "unsupported: VirtualTree (a VirtualList window is not a disclosure tree)";
const STATIC_TREE_50K_UNSUPPORTED: &str =
    "unsupported: StaticTree 50k (incomparable vs Nana construction-only)";
const UNSUPPORTED_MUTATION_KIND: &str = "unsupported: Mutation Visibility/Transform/Accessibility (no Nana hidden/PaintTransform/a11y analog)";

#[derive(Debug, Deserialize)]
struct ScenarioFile {
    schema_version: u32,
    id: String,
    kind: String,
    params: serde_json::Map<String, Value>,
}

struct Args {
    scenario: PathBuf,
    output: Option<PathBuf>,
    warmup: usize,
    iterations: usize,
}

struct Host {
    _cx: TestAppContext,
    visual: VisualTestContext,
}

struct FrameSamples {
    view_ms: Vec<f64>,
    layout_ms: Vec<f64>,
    draw_ms: Vec<f64>,
    cpu_ms: Vec<f64>,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn run() -> Result<u8, String> {
    let args = parse_args()?;
    let scenario = load_scenario(&args.scenario)?;
    if scenario.schema_version != 1 {
        return Err(format!(
            "unsupported scenario schema_version {}",
            scenario.schema_version
        ));
    }

    let report = match scenario.kind.as_str() {
        "StaticTree" => {
            let nodes = required_u64(&scenario.params, "nodes")?;
            if nodes == INCOMPARABLE_STATIC_TREE_NODES {
                unsupported_payload(&scenario, STATIC_TREE_50K_UNSUPPORTED)
            } else {
                benchmark_static_tree(&scenario.id, nodes, args.warmup, args.iterations)?
            }
        }
        "Mutation" => benchmark_mutation(&scenario, args.warmup, args.iterations)?,
        "Hover" => benchmark_hover(&scenario, args.warmup, args.iterations)?,
        "VirtualList" => benchmark_virtual_list(&scenario, args.warmup, args.iterations)?,
        "Table" => benchmark_table(&scenario, args.warmup, args.iterations)?,
        "DockWorkspace" => unsupported_payload(&scenario, DOCK_UNSUPPORTED),
        "TextEditor" => unsupported_payload(&scenario, TEXT_EDITOR_UNSUPPORTED),
        "Animation" => unsupported_payload(&scenario, ANIMATION_UNSUPPORTED),
        "Ime" => unsupported_payload(&scenario, IME_UNSUPPORTED),
        "Overlay" => unsupported_payload(&scenario, OVERLAY_UNSUPPORTED),
        "GpuScene" => unsupported_payload(&scenario, GPU_SCENE_UNSUPPORTED),
        "VirtualTree" => unsupported_payload(&scenario, VIRTUAL_TREE_UNSUPPORTED),
        other => unsupported_payload(
            &scenario,
            &format!(
                "gpui-scenario-bench has no same-scenario mapping for kind={other}. \
                 Fake GPUI numbers are forbidden."
            ),
        ),
    };

    let status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error");
    write_payload(&args.output, &report)?;
    Ok(if status == "ok" {
        EXIT_OK
    } else if status == "unsupported" {
        EXIT_UNSUPPORTED
    } else {
        EXIT_ERROR
    })
}

fn required_u64(params: &serde_json::Map<String, Value>, key: &str) -> Result<usize, String> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .map(|value| value as usize)
        .ok_or_else(|| format!("params.{key} must be a positive integer"))
}

fn parse_args() -> Result<Args, String> {
    let mut arguments = std::env::args_os().skip(1);
    let mut scenario = None;
    let mut output = None;
    let mut warmup = DEFAULT_WARMUP;
    let mut iterations = DEFAULT_ITERATIONS;
    while let Some(flag) = arguments.next() {
        match flag.to_str() {
            Some("--scenario") => {
                scenario =
                    Some(PathBuf::from(arguments.next().ok_or(
                        "--scenario requires a path to perf/scenarios/<id>.json",
                    )?));
            }
            Some("--output") => {
                output = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or("--output requires a destination path")?,
                ));
            }
            Some("--warmup") => {
                warmup = parse_usize(
                    arguments.next().ok_or("--warmup requires a count")?,
                    "--warmup",
                )?;
            }
            Some("--iterations") => {
                iterations = parse_usize(
                    arguments.next().ok_or("--iterations requires a count")?,
                    "--iterations",
                )?;
            }
            Some(other) => {
                return Err(format!(
                    "unsupported argument `{other}`; expected --scenario <path> [--output <path>]"
                ));
            }
            None => return Err("arguments must be valid UTF-8".to_owned()),
        }
    }
    let scenario = scenario.ok_or("provide --scenario <path>")?;
    if iterations == 0 {
        return Err("--iterations must be greater than 0".to_owned());
    }
    Ok(Args {
        scenario,
        output,
        warmup,
        iterations,
    })
}

fn parse_usize(value: std::ffi::OsString, flag: &str) -> Result<usize, String> {
    value
        .to_str()
        .ok_or_else(|| format!("{flag} must be UTF-8"))?
        .parse()
        .map_err(|_| format!("{flag} must be a non-negative integer"))
}

fn load_scenario(path: &Path) -> Result<ScenarioFile, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn write_payload(path: &Option<PathBuf>, payload: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(payload)
        .map_err(|error| format!("failed to serialize report: {error}"))?;
    match path {
        None => {
            println!("{text}");
            Ok(())
        }
        Some(path) => {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
            }
            fs::write(path, format!("{text}\n"))
                .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
            println!("{}", path.display());
            Ok(())
        }
    }
}

fn unsupported_payload(scenario: &ScenarioFile, reason: &str) -> Value {
    json!({
        "source": "gpui-scenario-bench",
        "status": "unsupported",
        "scenario_id": scenario.id,
        "kind": scenario.kind,
        "unsupported_reason": reason,
    })
}

fn create_host() -> Host {
    create_host_sized(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
}

fn create_host_sized(width: u32, height: u32) -> Host {
    let mut cx = TestAppContext::single();
    let window = cx.add_window(|window, _cx| {
        window.resize(size(px(width as f32), px(height as f32)));
        EmptyView
    });
    let visual = VisualTestContext::from_window(*window.deref(), &cx);
    visual.simulate_resize(size(px(width as f32), px(height as f32)));
    Host { _cx: cx, visual }
}

fn available_space(width: u32, height: u32) -> gpui::Size<AvailableSpace> {
    size(
        AvailableSpace::Definite(px(width as f32)),
        AvailableSpace::Definite(px(height as f32)),
    )
}

fn paint_tree(host: &mut Host, tree: AnyElement, width: u32, height: u32) -> (f64, f64) {
    let layout_slot = Cell::new(0.0);
    let draw_slot = Cell::new(0.0);
    host.visual.update(|window, cx| {
        let mut element = tree;
        let laid_out = Instant::now();
        element.layout_as_root(available_space(width, height), window, cx);
        layout_slot.set(elapsed_ms(laid_out));
        let drawn = Instant::now();
        element.prepaint_at(point(px(0.), px(0.)), window, cx);
        element.paint(window, cx);
        draw_slot.set(elapsed_ms(drawn));
    });
    (layout_slot.get(), draw_slot.get())
}

fn merge_meta(payload: &mut Value, warmup: usize, iterations: usize, width: u32, height: u32) {
    if let Value::Object(map) = payload {
        map.insert("viewport".into(), json!([width, height]));
        map.insert("scale".into(), json!(1.0));
        map.insert("backend".into(), json!("gpui-test-app-context"));
        map.insert("gpu_present".into(), json!(false));
        map.insert("warmup_iterations".into(), json!(warmup));
        map.insert("iterations".into(), json!(iterations));
        map.insert(
            "adapter".into(),
            json!({
                "name": "gpui TestAppContext / TestWindow",
                "crate": "gpui",
                "version": "0.2.2",
                "device_type": "test-platform",
                "backend": "none",
            }),
        );
        map.insert(
            "machine_identity".into(),
            json!({ "fixed_benchmark_machine": false }),
        );
    }
}

fn with_timings(mut payload: Value, samples: &FrameSamples) -> Value {
    payload["view_construction_ms"] = percentiles(&samples.view_ms);
    payload["layout_ms"] = percentiles(&samples.layout_ms);
    payload["draw_ms"] = percentiles(&samples.draw_ms);
    payload["cpu_frame_ms"] = percentiles(&samples.cpu_ms);
    payload
}

fn record_frame(
    samples: &mut FrameSamples,
    warmup: usize,
    iteration: usize,
    view: f64,
    layout: f64,
    draw: f64,
) {
    if iteration >= warmup {
        samples.view_ms.push(view);
        samples.layout_ms.push(layout);
        samples.draw_ms.push(draw);
        samples.cpu_ms.push(view + layout + draw);
    }
}

fn empty_samples(iterations: usize) -> FrameSamples {
    FrameSamples {
        view_ms: Vec::with_capacity(iterations),
        layout_ms: Vec::with_capacity(iterations),
        draw_ms: Vec::with_capacity(iterations),
        cpu_ms: Vec::with_capacity(iterations),
    }
}

fn benchmark_static_tree(
    scenario_id: &str,
    nodes: usize,
    warmup: usize,
    iterations: usize,
) -> Result<Value, String> {
    let mut host = create_host();
    let mut samples = empty_samples(iterations);

    for iteration in 0..(warmup + iterations) {
        let constructed = Instant::now();
        let tree = static_tree(nodes);
        let view = elapsed_ms(constructed);
        let (layout, draw) = paint_tree(&mut host, tree, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        record_frame(&mut samples, warmup, iteration, view, layout, draw);
    }

    let mut payload = json!({
        "source": "gpui-scenario-bench",
        "status": "ok",
        "scenario_id": scenario_id,
        "kind": "StaticTree",
        "params": { "nodes": nodes },
        "nodes": nodes,
        "tree": tree_provenance(nodes),
        "notes": [
            "Shared complete-binary-heap StaticTree. present_ms and frames_after_idle omitted (TestWindow / TestPlatform cannot observe them).",
        ],
    });
    merge_meta(
        &mut payload,
        warmup,
        iterations,
        VIEWPORT_WIDTH,
        VIEWPORT_HEIGHT,
    );
    Ok(with_timings(payload, &samples))
}

fn benchmark_mutation(
    scenario: &ScenarioFile,
    warmup: usize,
    iterations: usize,
) -> Result<Value, String> {
    let nodes = required_u64(&scenario.params, "tree_nodes")?;
    if nodes != REQUIRED_MUTATION_NODES {
        return Ok(unsupported_payload(
            scenario,
            &format!(
                "Mutation must use tree_nodes={REQUIRED_MUTATION_NODES} (catalog 5k single-node). \
                 Got tree_nodes={nodes}. Refusing to substitute another tree size."
            ),
        ));
    }
    let kind_name = scenario
        .params
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "Mutation.params.kind must be a string".to_owned())?;
    let kind = MutationKind::parse(kind_name)?;
    if !kind.is_same_scenario() {
        return Ok(unsupported_payload(scenario, UNSUPPORTED_MUTATION_KIND));
    }
    let target = mutation_target(kind, nodes);
    let mut host = create_host();
    let mut samples = empty_samples(iterations);

    for iteration in 0..(warmup + iterations) {
        let spec = MutationSpec {
            kind,
            target,
            even: iteration % 2 == 0,
        };
        let constructed = Instant::now();
        let tree = mutation_tree(nodes, spec);
        let view = elapsed_ms(constructed);
        let (layout, draw) = paint_tree(&mut host, tree, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        record_frame(&mut samples, warmup, iteration, view, layout, draw);
    }

    let mut payload = json!({
        "source": "gpui-scenario-bench",
        "status": "ok",
        "scenario_id": scenario.id,
        "kind": "Mutation",
        "params": { "tree_nodes": nodes, "kind": kind.as_str() },
        "nodes": nodes,
        "tree": tree_provenance(nodes),
        "mutation": {
            "kind": kind.as_str(),
            "target_index": target,
            "single_node": true,
        },
        "notes": [
            "Single-node mutation on the shared heap. No layout_nodes; those invariants stay not-evaluable.",
        ],
    });
    merge_meta(
        &mut payload,
        warmup,
        iterations,
        VIEWPORT_WIDTH,
        VIEWPORT_HEIGHT,
    );
    Ok(with_timings(payload, &samples))
}

fn benchmark_hover(
    scenario: &ScenarioFile,
    warmup: usize,
    iterations: usize,
) -> Result<Value, String> {
    let nodes = required_u64(&scenario.params, "nodes")?;
    if nodes != REQUIRED_HOVER_NODES {
        return Ok(unsupported_payload(
            scenario,
            &format!(
                "Hover must use nodes={REQUIRED_HOVER_NODES}. Got nodes={nodes}. \
                 Refusing to substitute a smaller tree."
            ),
        ));
    }
    let last = nodes;
    let previous = nodes.saturating_sub(1).max(1);
    let mut host = create_host();
    let mut samples = empty_samples(iterations);

    for iteration in 0..(warmup + iterations) {
        let hovered = if iteration % 2 == 0 {
            Some(last)
        } else {
            Some(previous)
        };
        let constructed = Instant::now();
        let tree = hover_tree(nodes, hovered);
        let view = elapsed_ms(constructed);
        let (layout, draw) = paint_tree(&mut host, tree, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        record_frame(&mut samples, warmup, iteration, view, layout, draw);
    }

    let mut payload = json!({
        "source": "gpui-scenario-bench",
        "status": "ok",
        "scenario_id": scenario.id,
        "kind": "Hover",
        "params": { "nodes": nodes, "size_change": false },
        "nodes": nodes,
        "tree": tree_provenance(nodes),
        "hover": {
            "targets": [previous, last],
            "size_change": false,
        },
        "notes": [
            "10k heap; hover toggles the last two nodes. No layout_nodes; hover_without_size_change stays not-evaluable.",
        ],
    });
    merge_meta(
        &mut payload,
        warmup,
        iterations,
        VIEWPORT_WIDTH,
        VIEWPORT_HEIGHT,
    );
    Ok(with_timings(payload, &samples))
}

fn benchmark_virtual_list(
    scenario: &ScenarioFile,
    warmup: usize,
    iterations: usize,
) -> Result<Value, String> {
    let items = required_u64(&scenario.params, "items")?;
    let visible = required_u64(&scenario.params, "visible")?;
    let overscan = scenario
        .params
        .get("overscan")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| "VirtualList.params.overscan must be a non-negative integer".to_owned())?;
    let text_len = required_u64(&scenario.params, "text_len")?;
    let item_extent = scenario
        .params
        .get("item_extent_px")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0)
        .map(|value| value as f32)
        .ok_or_else(|| "VirtualList.params.item_extent_px must be a positive number".to_owned())?;

    let bound = live_ui_entities_bound(visible, overscan);
    let mut host = create_host();
    let mut samples = empty_samples(iterations);
    let mut window_ms = Vec::with_capacity(iterations);
    let mut last_range: Range<usize> = 0..0;

    for iteration in 0..(warmup + iterations) {
        let scroll = 120.0 + (iteration as f32 * 13.0) % 4_000.0;
        let windowed = Instant::now();
        let range = virtual_window(items, scroll, visible, overscan, item_extent);
        let window = elapsed_ms(windowed);
        if range.len() > bound {
            return Err(format!(
                "virtual list materialized {} widgets, above live bound {bound} \
                 (visible={visible}, overscan={overscan}). Refusing to emit a fake full-list run.",
                range.len()
            ));
        }
        if range.len() == items && items > bound {
            return Err(
                "virtual list materialized every logical row; that is not Nana virtualization"
                    .to_owned(),
            );
        }
        last_range = range.clone();
        let leading = range.start as f32 * item_extent;
        let trailing = (items.saturating_sub(range.end)) as f32 * item_extent;

        let constructed = Instant::now();
        let tree = virtual_list_view(range, text_len, item_extent, leading, trailing);
        let view = elapsed_ms(constructed);
        let (layout, draw) = paint_tree(&mut host, tree, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        record_frame(&mut samples, warmup, iteration, view, layout, draw);
        if iteration >= warmup {
            window_ms.push(window);
        }
    }

    let live = last_range.len();
    let mut payload = json!({
        "source": "gpui-scenario-bench",
        "status": "ok",
        "scenario_id": scenario.id,
        "kind": "VirtualList",
        "params": {
            "items": items,
            "visible": visible,
            "overscan": overscan,
            "text_len": text_len,
            "item_extent_px": item_extent,
        },
        "virtualization": {
            "logical_items": items,
            "visible": visible,
            "overscan": overscan,
            "item_extent_px": item_extent,
            "live_ui_entities": live,
            "live_ui_entities_bound": bound,
            "materialized_widgets": live,
            "last_window": { "start": last_range.start, "end": last_range.end },
        },
        "work_counters": {
            "live_ui_entities": live,
            "live_ui_entities_bound": bound,
            "visible_rows": visible,
            "overscan_rows": overscan,
        },
        "window_ms": percentiles(&window_ms),
        "notes": [
            "Materializes only the catalog visible+overscan window, not logical_items widgets.",
        ],
    });
    merge_meta(
        &mut payload,
        warmup,
        iterations,
        VIEWPORT_WIDTH,
        VIEWPORT_HEIGHT,
    );
    Ok(with_timings(payload, &samples))
}

fn benchmark_table(
    scenario: &ScenarioFile,
    warmup: usize,
    iterations: usize,
) -> Result<Value, String> {
    let rows = required_u64(&scenario.params, "rows")?;
    let columns = required_u64(&scenario.params, "columns")?;
    let visible_rows = required_u64(&scenario.params, "visible_rows")?;
    let visible_columns = required_u64(&scenario.params, "visible_columns")?;
    let overscan_rows = scenario
        .params
        .get("overscan_rows")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| "Table.params.overscan_rows must be a non-negative integer".to_owned())?;
    let overscan_columns = scenario
        .params
        .get("overscan_columns")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| "Table.params.overscan_columns must be a non-negative integer".to_owned())?;
    let short_cell_len = required_u64(&scenario.params, "short_cell_len")?;
    let wrapped_cells = required_u64(&scenario.params, "wrapped_cells")?;
    let wrapped_cell_len = required_u64(&scenario.params, "wrapped_cell_len")?;
    if short_cell_len != TABLE_SHORT_CELL_LEN
        || wrapped_cells != TABLE_WRAPPED_CELLS
        || wrapped_cell_len != TABLE_WRAPPED_CELL_LEN
    {
        return Ok(unsupported_payload(
            scenario,
            &format!(
                "Table cell generator must match Nana text_table_cell \
                 (short_cell_len={TABLE_SHORT_CELL_LEN}, wrapped_cells={TABLE_WRAPPED_CELLS}, \
                 wrapped_cell_len={TABLE_WRAPPED_CELL_LEN}); catalog has \
                 {short_cell_len}/{wrapped_cells}/{wrapped_cell_len}."
            ),
        ));
    }

    let bound = table_live_ui_entities_bound(
        visible_rows,
        overscan_rows,
        visible_columns,
        overscan_columns,
    );
    let viewport_w = (visible_columns as f32 * TABLE_COLUMN_EXTENT_PX).round() as u32;
    let viewport_h = (visible_rows as f32 * TABLE_ROW_EXTENT_PX).round() as u32;
    let mut host = create_host_sized(viewport_w.max(1), viewport_h.max(1));
    let mut samples = empty_samples(iterations);
    let mut window_ms = Vec::with_capacity(iterations);
    let mut last_row_range: Range<usize> = 0..0;
    let mut last_col_range: Range<usize> = 0..0;
    let mut last_live = 0usize;

    for iteration in 0..(warmup + iterations) {
        let scroll_y = if iteration.is_multiple_of(2) {
            120.0
        } else {
            140.0
        };
        let scroll_x = if iteration.is_multiple_of(2) {
            0.0
        } else {
            80.0
        };
        let windowed = Instant::now();
        let row_range = table_axis_window(
            rows,
            scroll_y,
            visible_rows,
            overscan_rows,
            TABLE_ROW_EXTENT_PX,
        );
        let col_range = table_axis_window(
            columns,
            scroll_x,
            visible_columns,
            overscan_columns,
            TABLE_COLUMN_EXTENT_PX,
        );
        let window = elapsed_ms(windowed);
        let live_cells = row_range.len().saturating_mul(col_range.len());
        let live = row_range.len().saturating_add(live_cells);
        if live_cells == rows.saturating_mul(columns) && rows.saturating_mul(columns) > bound {
            return Err(
                "virtual table materialized every logical cell; that is not Nana virtualization"
                    .to_owned(),
            );
        }
        if live > bound {
            return Err(format!(
                "virtual table live_ui_entities={live} exceeds bound {bound} \
                 (visible={visible_rows}x{visible_columns}, overscan={overscan_rows}x{overscan_columns})"
            ));
        }
        last_row_range = row_range.clone();
        last_col_range = col_range.clone();
        last_live = live;
        let leading = row_range.start as f32 * TABLE_ROW_EXTENT_PX;
        let trailing = (rows.saturating_sub(row_range.end)) as f32 * TABLE_ROW_EXTENT_PX;

        let constructed = Instant::now();
        let tree = virtual_table_view(
            row_range,
            col_range,
            TABLE_ROW_EXTENT_PX,
            TABLE_COLUMN_EXTENT_PX,
            leading,
            trailing,
        );
        let view = elapsed_ms(constructed);
        let (layout, draw) = paint_tree(&mut host, tree, viewport_w.max(1), viewport_h.max(1));
        record_frame(&mut samples, warmup, iteration, view, layout, draw);
        if iteration >= warmup {
            window_ms.push(window);
        }
    }

    let mut payload = json!({
        "source": "gpui-scenario-bench",
        "status": "ok",
        "scenario_id": scenario.id,
        "kind": "Table",
        "params": {
            "rows": rows,
            "columns": columns,
            "visible_rows": visible_rows,
            "visible_columns": visible_columns,
            "overscan_rows": overscan_rows,
            "overscan_columns": overscan_columns,
            "short_cell_len": short_cell_len,
            "wrapped_cells": wrapped_cells,
            "wrapped_cell_len": wrapped_cell_len,
            "row_extent_px": TABLE_ROW_EXTENT_PX,
            "column_extent_px": TABLE_COLUMN_EXTENT_PX,
        },
        "virtualization": {
            "logical_rows": rows,
            "logical_columns": columns,
            "visible_rows": visible_rows,
            "visible_columns": visible_columns,
            "overscan_rows": overscan_rows,
            "overscan_columns": overscan_columns,
            "row_extent_px": TABLE_ROW_EXTENT_PX,
            "column_extent_px": TABLE_COLUMN_EXTENT_PX,
            "live_ui_entities": last_live,
            "live_ui_entities_bound": bound,
            "last_row_window": { "start": last_row_range.start, "end": last_row_range.end },
            "last_col_window": { "start": last_col_range.start, "end": last_col_range.end },
        },
        "work_counters": {
            "live_ui_entities": last_live,
            "live_ui_entities_bound": bound,
            "visible_rows": visible_rows,
            "overscan_rows": overscan_rows,
            "visible_columns": visible_columns,
            "overscan_columns": overscan_columns,
            "wrapped_cells": wrapped_cells,
            "wrapped_cell_len": wrapped_cell_len,
            "short_cell_len": short_cell_len,
        },
        "window_ms": percentiles(&window_ms),
        "notes": [
            "Materializes only the catalog table window. No text_shaped / glyph_cache_*.",
        ],
    });
    merge_meta(
        &mut payload,
        warmup,
        iterations,
        viewport_w.max(1),
        viewport_h.max(1),
    );
    Ok(with_timings(payload, &samples))
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

fn percentiles(samples: &[f64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    json!({
        "p50": percentile(&sorted, 50.0),
        "p95": percentile(&sorted, 95.0),
        "p99": percentile(&sorted, 99.0),
        "min": sorted.first().copied().unwrap_or(0.0),
        "max": sorted.last().copied().unwrap_or(0.0),
    })
}

fn percentile(sorted: &[f64], percent: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (percent / 100.0) * (sorted.len() - 1) as f64;
    let low = rank.floor() as usize;
    let high = rank.ceil() as usize;
    if low == high {
        sorted[low]
    } else {
        let weight = rank - low as f64;
        sorted[low] * (1.0 - weight) + sorted[high] * weight
    }
}
