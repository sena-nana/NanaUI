//! Headless Iced runner for NanaUI Issue #8 / #12 shared Scenario JSON.
//!
//! StaticTree / Mutation / Hover share Nana's complete-binary-heap
//! (`parent(i)=i/2`, root `1`, element-div). VirtualList materializes only the
//! catalog window. Table materializes only the catalog table window. StaticTree
//! 50k is refused: Nana is construction-only there. Animation / Ime / Dock /
//! Overlay / TextEditor / GpuScene stay unsupported. Fake timings are not emitted.
mod tree;

use iced::{Color, Event, Theme};
use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::core::mouse;
use iced_winit::core::renderer;
use iced_winit::core::shell;
use iced_winit::core::window;
use iced_winit::futures::futures::executor;
use iced_winit::runtime::user_interface::{self, UserInterface};

use serde::Deserialize;
use serde_json::{Value, json};

use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use crate::tree::{
    BenchElement, MutationKind, MutationSpec, TABLE_COLUMN_EXTENT_PX, TABLE_ROW_EXTENT_PX,
    TABLE_SHORT_CELL_LEN, TABLE_WRAPPED_CELL_LEN, TABLE_WRAPPED_CELLS, busy_pulse, hover_tree,
    live_ui_entities_bound, mutation_target, mutation_tree, static_tree, table_axis_window,
    table_live_ui_entities_bound, tree_provenance, virtual_list_view, virtual_table_view,
    virtual_window,
};
const VIEWPORT_WIDTH: u32 = 900;
const VIEWPORT_HEIGHT: u32 = 640;
const SCALE: f32 = 1.0;
const DEFAULT_WARMUP: usize = 5;
const DEFAULT_ITERATIONS: usize = 20;
const REQUIRED_HOVER_NODES: usize = 10_000;
const REQUIRED_MUTATION_NODES: usize = 5_000;
const INCOMPARABLE_STATIC_TREE_NODES: usize = 50_000;
/// Host ticks after settle used to count UI-requested frames. Not a timer window.
const IDLE_FRAME_OBSERVE_TICKS: usize = 8;

const EXIT_OK: u8 = 0;
const EXIT_ERROR: u8 = 1;
const EXIT_UNSUPPORTED: u8 = 2;

const ANIMATION_UNSUPPORTED: &str = "\
Iced has no Runtime animation scheduler. Nana catalog Animation measures \
next_animation_deadline idle/scheduled plus advance_animations with \
due_animation_samples=1 on an isolated UiWorld. A tweening widget is not that work.";
const IME_UNSUPPORTED: &str = "\
Iced has no set_ime_preedit / commit_ime. Nana Ime measures those Runtime calls \
on a focused TextInput for latin/zh/ja/ko. OS IME UI and text_input typing are \
not that dirty work.";
const OVERLAY_UNSUPPORTED: &str = "\
Iced has no OverlayHost activate_overlay/dismiss_overlay or toggle_popover. \
Nana Overlay measures those APIs for Tooltip, Menu, Dialog, and Popover. \
Always-on tooltips or centered containers are not that dirty work.";
const GPU_SCENE_UNSUPPORTED: &str = "\
Iced has no nana-gpu-scene-benchmark UiOnly path. GpuScene / Live2D stay \
unsupported; do not invent GPU upload or Live2D zeros.";
const DOCK_UNSUPPORTED: &str = "\
Iced pane_grid topology/axis/0.50-0.55/1280x800/panes=8 is not Nana catalog Dock. \
Nana adjust_focused_dock_split calls assemble_dock and rebuilds chrome \
(titles/strips/handles). Chrome-less incremental splits are not that dirty work. \
Unsupported until Iced resize rebuilds equivalent chrome.";
const TEXT_EDITOR_UNSUPPORTED: &str = "\
Iced cannot observe Nana replace_text_area_selection then drain_text on a 100k \
buffer. Timing only view + reused UserInterface::build + draw after an untimed \
edit is a cached no-op, not that dirty work. Unsupported until the analog of \
edit+drain dirties the timed frame. Do not invent WorkCounters.text_shaped.";

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
    device: wgpu::Device,
    renderer: Renderer,
    viewport: Viewport,
    texture_view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    target_width: u32,
    target_height: u32,
    adapter_name: String,
    adapter_backend: String,
    adapter_device_type: String,
}

struct FrameSamples {
    view_ms: Vec<f64>,
    layout_ms: Vec<f64>,
    draw_ms: Vec<f64>,
    present_ms: Vec<f64>,
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
                unsupported_payload(
                    &scenario,
                    "StaticTree 50k is not comparable: Nana nana-runtime-benchmark maps this id as \
                     kind=construction (enqueue/commit/paint/hover only), not a full systems pass. \
                     A full Iced layout+draw of 50k nodes would be a different workload. \
                     Unsupported until both sides share the same work definition. \
                     Do not silently compare construction-only Nana vs full Iced 50k.",
                )
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
        other => unsupported_payload(
            &scenario,
            &format!(
                "iced-scenario-bench has no same-scenario mapping for kind={other}. \
                 Gallery ui-benchmark is not a substitute."
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
        "source": "iced-scenario-bench",
        "status": "unsupported",
        "scenario_id": scenario.id,
        "kind": scenario.kind,
        "unsupported_reason": reason,
    })
}

fn create_host() -> Result<Host, String> {
    create_host_sized(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
}

fn create_host_sized(width: u32, height: u32) -> Result<Host, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|error| format!("iced-scenario-bench needs a real WGPU adapter: {error}"))?;
    let adapter_info = adapter.get_info();
    let (device, queue) = executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("iced-scenario-bench"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .map_err(|error| format!("iced-scenario-bench failed to create a WGPU device: {error}"))?;

    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let engine = Engine::new(
        &adapter,
        device.clone(),
        queue.clone(),
        format,
        None,
        Shell::headless(),
    );
    let renderer = Renderer::new(engine, renderer::Settings::default());
    let viewport = Viewport::with_physical_size(
        iced_wgpu::core::Size::new(width, height),
        renderer::Scale {
            window: SCALE,
            application: 1.0,
        },
    );
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("iced-scenario-bench target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(Host {
        device,
        renderer,
        viewport,
        texture_view,
        format,
        target_width: width,
        target_height: height,
        adapter_name: adapter_info.name,
        adapter_backend: format!("{:?}", adapter_info.backend),
        adapter_device_type: format!("{:?}", adapter_info.device_type),
    })
}

fn present(host: &mut Host) -> f64 {
    let presented = Instant::now();
    let submission = host.renderer.present(
        Some(Color::BLACK),
        host.format,
        &host.texture_view,
        &host.viewport,
    );
    let _ = host.device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    });
    elapsed_ms(presented)
}

fn draw_ui(host: &mut Host, user_interface: &mut UserInterface<(), Theme, Renderer>) -> f64 {
    let drawn = Instant::now();
    user_interface.draw(
        &mut host.renderer,
        &Theme::Dark,
        &renderer::Style {
            text_color: Color::WHITE,
        },
        mouse::Cursor::Unavailable,
    );
    elapsed_ms(drawn)
}

fn adapter_json(host: &Host) -> Value {
    json!({
        "name": host.adapter_name,
        "backend": host.adapter_backend,
        "device_type": host.adapter_device_type,
    })
}

fn merge_meta(payload: &mut Value, warmup: usize, iterations: usize, host: &Host) {
    if let Value::Object(map) = payload {
        map.insert(
            "viewport".into(),
            json!([host.target_width, host.target_height]),
        );
        map.insert("scale".into(), json!(SCALE));
        map.insert("backend".into(), json!("wgpu"));
        map.insert("antialiasing".into(), json!("none"));
        map.insert("warmup_iterations".into(), json!(warmup));
        map.insert("iterations".into(), json!(iterations));
        map.insert("adapter".into(), adapter_json(host));
        map.insert("gpu_present".into(), json!(true));
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
    payload["present_ms"] = percentiles(&samples.present_ms);
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
    present_ms: f64,
) {
    if iteration >= warmup {
        samples.view_ms.push(view);
        samples.layout_ms.push(layout);
        samples.draw_ms.push(draw);
        samples.present_ms.push(present_ms);
        samples.cpu_ms.push(view + layout + draw);
    }
}

fn empty_samples(iterations: usize) -> FrameSamples {
    FrameSamples {
        view_ms: Vec::with_capacity(iterations),
        layout_ms: Vec::with_capacity(iterations),
        draw_ms: Vec::with_capacity(iterations),
        present_ms: Vec::with_capacity(iterations),
        cpu_ms: Vec::with_capacity(iterations),
    }
}

fn benchmark_static_tree(
    scenario_id: &str,
    nodes: usize,
    warmup: usize,
    iterations: usize,
) -> Result<Value, String> {
    let mut host = create_host()?;
    let busy_frames = count_frames_after_idle(&mut host, busy_pulse(), IDLE_FRAME_OBSERVE_TICKS);
    if busy_frames == 0 {
        return Err(
            "Iced headless UserInterface::update did not observe a UI-requested redraw from a \
             busy widget after settle. Refusing to emit frames_after_idle=0."
                .to_owned(),
        );
    }
    let frames_after_idle =
        count_frames_after_idle(&mut host, static_tree(nodes), IDLE_FRAME_OBSERVE_TICKS);
    let mut samples = empty_samples(iterations);

    for iteration in 0..(warmup + iterations) {
        let constructed = Instant::now();
        let tree = static_tree(nodes);
        let view = elapsed_ms(constructed);

        let laid_out = Instant::now();
        let mut user_interface = UserInterface::build(
            tree,
            host.viewport.logical_size(),
            user_interface::Cache::default(),
            &mut host.renderer,
        );
        let layout = elapsed_ms(laid_out);
        let draw = draw_ui(&mut host, &mut user_interface);
        drop(user_interface.into_cache());
        let present_ms = present(&mut host);
        record_frame(
            &mut samples,
            warmup,
            iteration,
            view,
            layout,
            draw,
            present_ms,
        );
    }

    let mut payload = json!({
        "source": "iced-scenario-bench",
        "status": "ok",
        "scenario_id": scenario_id,
        "kind": "StaticTree",
        "params": { "nodes": nodes },
        "nodes": nodes,
        "tree": tree_provenance(nodes),
        "frames_after_idle": frames_after_idle,
        "busy_probe_frames": busy_frames,
        "idle_observe_ticks": IDLE_FRAME_OBSERVE_TICKS,
        "notes": [
            "StaticTree JSON only has params.nodes. Tree shape is the shared complete-binary-heap rule: parent(i)=i/2, root=1, element-div, no text.",
            "Same rule as nana-runtime-benchmark::tree_mutations. A flat column of N text leaves is not this tree.",
            "cpu_frame_ms is view construction + UserInterface::build (layout) + draw. present_ms is GPU submit wait and is not folded into cpu_frame_ms.",
            "frames_after_idle is the §8.1 idle-frame count (UserInterface redraw NextFrame/At after settle). A BusyPulse probe must be non-zero before 0 is emitted.",
            "Generated by: cargo run --release --locked --manifest-path engine/iced/Cargo.toml -p scenario-bench -- --scenario perf/scenarios/static-tree-100.json --output perf/fixtures/iced-scenario-static-tree-100.json",
        ],
    });
    merge_meta(&mut payload, warmup, iterations, &host);
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
        return Ok(unsupported_payload(
            scenario,
            "Iced scenario-bench has no same-scenario Visibility, Transform, or Accessibility. \
             Nana measure_single_node_mutations uses LayoutStyle.hidden, PaintTransform.e \
             {4|8}, and set_accessibility labels alpha/beta. Height-0+clip, Shadow offset, \
             or widget Id is not that dirty work. Unsupported until Iced applies the same kind.",
        ));
    }
    let target = mutation_target(kind, nodes);
    let mut host = create_host()?;
    let mut cache = user_interface::Cache::default();
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

        let laid_out = Instant::now();
        let mut user_interface = UserInterface::build(
            tree,
            host.viewport.logical_size(),
            cache,
            &mut host.renderer,
        );
        let layout = elapsed_ms(laid_out);
        let draw = draw_ui(&mut host, &mut user_interface);
        cache = user_interface.into_cache();
        let present_ms = present(&mut host);
        record_frame(
            &mut samples,
            warmup,
            iteration,
            view,
            layout,
            draw,
            present_ms,
        );
    }

    let mut payload = json!({
        "source": "iced-scenario-bench",
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
            "Same complete-binary-heap as Nana tree_mutations / Iced StaticTree. One node is mutated per sample; topology is unchanged.",
            "UserInterface cache is reused so the measured frame is an incremental rebuild, not a cold 5k construction.",
            "Iced has no WorkCounters.layout_nodes; paint-only / a11y layout invariants stay not-evaluable. Do not invent zeros.",
            "cpu_frame_ms is view construction + UserInterface::build + draw.",
        ],
    });
    merge_meta(&mut payload, warmup, iterations, &host);
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
    let mut host = create_host()?;
    let mut cache = user_interface::Cache::default();
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

        let laid_out = Instant::now();
        let mut user_interface = UserInterface::build(
            tree,
            host.viewport.logical_size(),
            cache,
            &mut host.renderer,
        );
        let layout = elapsed_ms(laid_out);
        let draw = draw_ui(&mut host, &mut user_interface);
        cache = user_interface.into_cache();
        let present_ms = present(&mut host);
        record_frame(
            &mut samples,
            warmup,
            iteration,
            view,
            layout,
            draw,
            present_ms,
        );
    }

    let mut payload = json!({
        "source": "iced-scenario-bench",
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
            "10k complete-binary-heap, same topology as Nana tree_mutations. Hover toggles style on the last two nodes, matching Nana set_pointer_hover between node(n) and node(n-1).",
            "This is a targeted hover-state change, not a 10k hit-test walk and not a smaller tree.",
            "Iced has no WorkCounters.layout_nodes; hover_without_size_change stays not-evaluable. Do not invent layout_nodes=0.",
            "cpu_frame_ms is view construction + UserInterface::build + draw with a reused cache.",
        ],
    });
    merge_meta(&mut payload, warmup, iterations, &host);
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
    let mut host = create_host()?;
    let mut cache = user_interface::Cache::default();
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

        let laid_out = Instant::now();
        let mut user_interface = UserInterface::build(
            tree,
            host.viewport.logical_size(),
            cache,
            &mut host.renderer,
        );
        let layout = elapsed_ms(laid_out);
        let draw = draw_ui(&mut host, &mut user_interface);
        cache = user_interface.into_cache();
        let present_ms = present(&mut host);
        record_frame(
            &mut samples,
            warmup,
            iteration,
            view,
            layout,
            draw,
            present_ms,
        );
        if iteration >= warmup {
            window_ms.push(window);
        }
    }

    let live = last_range.len();
    let mut payload = json!({
        "source": "iced-scenario-bench",
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
            "Iced materializes only visible+overscan rows from catalog params, plus leading/trailing spacers. This is not 10k real widgets.",
            "Catalog window: visible×item_extent viewport, overscan items×item_extent. Nana runner passes the same --list-viewport-px / --list-overscan-px / --list-item-extent-px (10k/100k: 800 / 160 / 20).",
            "live_ui_entities is the materialized item widget count, matching Nana list children after materialize_virtual_list.",
            "cpu_frame_ms is view construction + UserInterface::build + draw of the live window only.",
        ],
    });
    merge_meta(&mut payload, warmup, iterations, &host);
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
    let mut host = create_host_sized(viewport_w.max(1), viewport_h.max(1))?;
    let mut cache = user_interface::Cache::default();
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

        let laid_out = Instant::now();
        let mut user_interface = UserInterface::build(
            tree,
            host.viewport.logical_size(),
            cache,
            &mut host.renderer,
        );
        let layout = elapsed_ms(laid_out);
        let draw = draw_ui(&mut host, &mut user_interface);
        cache = user_interface.into_cache();
        let present_ms = present(&mut host);
        record_frame(
            &mut samples,
            warmup,
            iteration,
            view,
            layout,
            draw,
            present_ms,
        );
        if iteration >= warmup {
            window_ms.push(window);
        }
    }

    let mut payload = json!({
        "source": "iced-scenario-bench",
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
            "Iced materializes only the catalog table window (visible+overscan rows and columns), not rows×columns widgets.",
            "Cell generator matches Nana text_table_cell: short_cell_len labels; column 0 of the first wrapped_cells rows in each 40-row band is wrapped_cell_len.",
            "live_ui_entities is mounted_rows + mounted_rows×mounted_columns, matching Nana virtual table children.",
            "Iced has no WorkCounters.text_shaped / glyph_cache_*; those stay omitted / not-evaluable.",
            "cpu_frame_ms is view construction + UserInterface::build + draw of the live window only.",
        ],
    });
    merge_meta(&mut payload, warmup, iterations, &host);
    Ok(with_timings(payload, &samples))
}

fn count_frames_after_idle(host: &mut Host, tree: BenchElement<'_>, ticks: usize) -> usize {
    let mut user_interface = UserInterface::build(
        tree,
        host.viewport.logical_size(),
        user_interface::Cache::default(),
        &mut host.renderer,
    );
    let _ = draw_ui(host, &mut user_interface);
    let _ = present(host);

    let mut frames = 0usize;
    let mut pending_redraw = true;
    let waker = shell::Waker::noop();
    let window = window::Headless;
    for _ in 0..ticks {
        let events = if pending_redraw {
            vec![Event::Window(window::Event::RedrawRequested(
                iced_winit::core::time::Instant::now(),
            ))]
        } else {
            Vec::new()
        };
        pending_redraw = false;
        let mut messages = shell::Bus::new();
        let (state, _) = user_interface.update(
            &window,
            &waker,
            &events,
            mouse::Cursor::Unavailable,
            &mut host.renderer,
            &mut messages,
        );
        match state {
            user_interface::State::Outdated => {
                frames += 1;
                pending_redraw = true;
            }
            user_interface::State::Updated { redraw_request, .. } => {
                if matches!(
                    redraw_request,
                    window::RedrawRequest::NextFrame | window::RedrawRequest::At(_)
                ) {
                    frames += 1;
                    let _ = draw_ui(host, &mut user_interface);
                    let _ = present(host);
                    pending_redraw = true;
                }
            }
        }
    }
    drop(user_interface.into_cache());
    frames
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
