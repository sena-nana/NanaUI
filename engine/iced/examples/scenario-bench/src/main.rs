//! Headless Iced runner for NanaUI Issue #8 / #12 shared Scenario JSON.
//!
//! StaticTree JSON only carries `params.nodes`. Hierarchy and kind come from the
//! shared generation rule also used by Nana `tree_mutations` in
//! `nana-runtime-benchmark.rs`: complete binary heap, `parent(i)=i/2`, root `1`,
//! `NodeKind::Element { tag: "div" }`, no text. N text leaves are not that tree.
//! Unsupported kinds exit 2. Fake timings are not emitted.
use iced::widget::{column, container, space};
use iced::{Color, Element, Theme};
use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::core::mouse;
use iced_winit::core::renderer;
use iced_winit::futures::futures::executor;
use iced_winit::runtime::user_interface::{self, UserInterface};

use serde::Deserialize;
use serde_json::{Value, json};

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

const VIEWPORT_WIDTH: u32 = 900;
const VIEWPORT_HEIGHT: u32 = 640;
const SCALE: f32 = 1.0;
const DEFAULT_WARMUP: usize = 5;
const DEFAULT_ITERATIONS: usize = 20;

const EXIT_OK: u8 = 0;
const EXIT_ERROR: u8 = 1;
const EXIT_UNSUPPORTED: u8 = 2;

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
    if scenario.kind != "StaticTree" {
        let payload = json!({
            "source": "iced-scenario-bench",
            "status": "unsupported",
            "scenario_id": scenario.id,
            "kind": scenario.kind,
            "unsupported_reason": format!(
                "iced-scenario-bench currently implements StaticTree only. \
                 {} is required by #8 / not implemented on engine/iced. \
                 Mutation, Hover, VirtualList, and Table stay exit 2.",
                scenario.kind
            ),
        });
        write_payload(&args.output, &payload)?;
        return Ok(EXIT_UNSUPPORTED);
    }
    let nodes = scenario
        .params
        .get("nodes")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "StaticTree.params.nodes must be a positive integer".to_owned())?
        as usize;

    let report = benchmark_static_tree(&scenario.id, nodes, args.warmup, args.iterations)?;
    write_payload(&args.output, &report)?;
    Ok(EXIT_OK)
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

/// Shared with Nana `tree_mutations`: parent of `index` is `index / 2`.
fn static_tree_parent(index: usize) -> Option<usize> {
    (index > 1).then_some(index / 2)
}

/// Materialize `1..=nodes` as nested Iced containers (div analogues).
fn static_tree<'a>(nodes: usize) -> Element<'a, (), Theme, Renderer> {
    static_tree_node(1, nodes)
}

fn static_tree_node<'a>(index: usize, nodes: usize) -> Element<'a, (), Theme, Renderer> {
    let mut children = Vec::new();
    let left = index * 2;
    let right = left + 1;
    if left <= nodes {
        children.push(static_tree_node(left, nodes));
    }
    if right <= nodes {
        children.push(static_tree_node(right, nodes));
    }
    if children.is_empty() {
        container(space().width(1).height(1)).into()
    } else {
        container(column(children)).into()
    }
}

fn sample_parents(nodes: usize) -> Value {
    let mut indexes = vec![1usize, 2, 3];
    if nodes >= 50 {
        indexes.push(50);
    }
    indexes.push(nodes);
    indexes.sort_unstable();
    indexes.dedup();
    Value::Array(
        indexes
            .into_iter()
            .filter(|index| *index >= 1 && *index <= nodes)
            .map(|index| {
                json!({
                    "index": index,
                    "parent": static_tree_parent(index),
                })
            })
            .collect(),
    )
}

fn benchmark_static_tree(
    scenario_id: &str,
    nodes: usize,
    warmup: usize,
    iterations: usize,
) -> Result<Value, String> {
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
    let mut renderer = Renderer::new(engine, renderer::Settings::default());
    let viewport = Viewport::with_physical_size(
        iced_wgpu::core::Size::new(VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
        renderer::Scale {
            window: SCALE,
            application: 1.0,
        },
    );
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("iced-scenario-bench target"),
        size: wgpu::Extent3d {
            width: VIEWPORT_WIDTH,
            height: VIEWPORT_HEIGHT,
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

    let mut view_ms = Vec::with_capacity(iterations);
    let mut layout_ms = Vec::with_capacity(iterations);
    let mut draw_ms = Vec::with_capacity(iterations);
    let mut present_ms = Vec::with_capacity(iterations);
    let mut cpu_ms = Vec::with_capacity(iterations);

    for iteration in 0..(warmup + iterations) {
        let constructed = Instant::now();
        let tree = static_tree(nodes);
        let view_construction = elapsed_ms(constructed);

        let laid_out = Instant::now();
        let mut user_interface = UserInterface::build(
            tree,
            viewport.logical_size(),
            user_interface::Cache::default(),
            &mut renderer,
        );
        let layout = elapsed_ms(laid_out);

        let drawn = Instant::now();
        user_interface.draw(
            &mut renderer,
            &Theme::Dark,
            &renderer::Style {
                text_color: Color::WHITE,
            },
            mouse::Cursor::Unavailable,
        );
        let draw = elapsed_ms(drawn);
        drop(user_interface.into_cache());

        let presented = Instant::now();
        let submission = renderer.present(Some(Color::BLACK), format, &texture_view, &viewport);
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        });
        let present = elapsed_ms(presented);

        if iteration >= warmup {
            view_ms.push(view_construction);
            layout_ms.push(layout);
            draw_ms.push(draw);
            present_ms.push(present);
            cpu_ms.push(view_construction + layout + draw);
        }
    }

    Ok(json!({
        "source": "iced-scenario-bench",
        "status": "ok",
        "scenario_id": scenario_id,
        "kind": "StaticTree",
        "params": { "nodes": nodes },
        "nodes": nodes,
        "tree": {
            "generation": "complete-binary-heap",
            "parent_rule": "parent(i)=i//2, root=1",
            "node_kind": "element-div",
            "text": null,
            "sample_parents": sample_parents(nodes),
        },
        "viewport": [VIEWPORT_WIDTH, VIEWPORT_HEIGHT],
        "scale": SCALE,
        "backend": "wgpu",
        "antialiasing": "none",
        "warmup_iterations": warmup,
        "iterations": iterations,
        "adapter": {
            "name": adapter_info.name,
            "backend": format!("{:?}", adapter_info.backend),
            "device_type": format!("{:?}", adapter_info.device_type),
        },
        "view_construction_ms": percentiles(&view_ms),
        "layout_ms": percentiles(&layout_ms),
        "draw_ms": percentiles(&draw_ms),
        "present_ms": percentiles(&present_ms),
        "cpu_frame_ms": percentiles(&cpu_ms),
        "notes": [
            "StaticTree JSON only has params.nodes. Tree shape is the shared complete-binary-heap rule: parent(i)=i/2, root=1, element-div, no text.",
            "Same rule as nana-runtime-benchmark::tree_mutations. A flat column of N text leaves is not this tree.",
            "cpu_frame_ms is view construction + UserInterface::build (layout) + draw. present_ms is GPU submit wait and is not folded into cpu_frame_ms.",
        ],
    }))
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
