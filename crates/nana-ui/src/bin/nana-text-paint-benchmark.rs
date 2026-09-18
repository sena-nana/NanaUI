//! Headless text-paint bench for Issue #98.
//!
//! What it measures is the frame a shell actually pays for: one that *rebatches*.
//! The painter already skips everything when nothing in the scene moved, so the
//! interesting question is what a text-heavy frame costs when something else on
//! screen did — an animation, a hover, a caret. Each workload below changes
//! exactly one thing and reports what the text path had to redo for it.
//!
//! The numbers that decide whether #98 landed are the work counters, not the
//! milliseconds: `text_instance_rebuilds`, `text_instance_upload_bytes` and
//! `glyph_resolve_requests` say whether a steady frame touched a glyph at all.
//! The timings are here because a structural win that costs more CPU is not a
//! win, not because a public CI runner can resolve 20 µs.
//!
//! ```bash
//! cargo run --release --locked -p nana-ui --features gpu \
//!     --bin nana-text-paint-benchmark -- --output target/performance/issue98/text-paint.json
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use nana_ui::runtime::{
    DocumentId, FlexDirection, FlexWrap, LayoutStyle, LayoutViewport, LengthSpec, MutationQueue,
    NodeKind, NodeStyle, RuntimeDocument, SemanticColorRole, StableNodeId, TextContent,
};
use nana_ui::{NanaTextShaper, ScenePaintViewport, SceneWgpuPainter};
use nana_ui_core::{PaintTransform, TransformOrigin};
use serde::Serialize;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const WARMUP_FRAMES: usize = 8;
/// One label's box. The viewport is sized from these so every label a cell
/// asks for is actually painted — a column taller than the screen would have
/// the scene cull the rest and every cell would measure the same forty rows.
const LABEL: [f32; 2] = [104.0, 20.0];
/// Distinct strings a repeated-label cell draws from. A table repeats its
/// cells, and a paint benchmark that gave every label its own string would
/// measure the shape cache instead of the paint path.
const DISTINCT_LABELS: usize = 64;

/// Label counts Issue #98 asks for.
const LABEL_GRID: [usize; 4] = [1, 100, 1_000, 10_000];
/// Frame counts standing in for 60 / 120 / 240 Hz of one second of animation.
const RATE_GRID: [usize; 3] = [60, 120, 240];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Workload {
    /// Nothing about the text changes; something else in the scene does, so the
    /// frame is rebatched. This is the row the whole issue is about.
    Static,
    /// `Static`, but every label has its own string. Beyond the shape cache's
    /// capacity this stops measuring paint and starts measuring reshaping,
    /// which is the point of keeping the row.
    StaticUnique,
    /// Every label's color cycles. No reshape, no raster, no instance.
    Color,
    /// The container fades. Presentation only.
    Opacity,
    /// The container rotates. Presentation only.
    Transform,
    /// One label in a hundred gets new text.
    Mutate,
}

impl Workload {
    fn id(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::StaticUnique => "static-unique",
            Self::Color => "color",
            Self::Opacity => "opacity",
            Self::Transform => "transform",
            Self::Mutate => "mutate-1pct",
        }
    }

    /// Whether the workload is an animation, and therefore worth running at
    /// each of [`RATE_GRID`].
    fn animated(self) -> bool {
        matches!(self, Self::Color | Self::Opacity | Self::Transform)
    }
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    adapter: String,
    cells: Vec<Cell>,
}

#[derive(Serialize)]
struct Cell {
    workload: String,
    labels: usize,
    /// Wrapper elements between the document and the labels.
    depth: usize,
    viewport: [u32; 2],
    frames: usize,
    /// Glyphs the visible labels resolve to, so a per-glyph reading is possible.
    live_glyphs: u64,
    /// Per sampled frame.
    counters: BTreeMap<String, f64>,
    /// `RuntimeDocument::flush`: extraction, layout and the scene delta. The
    /// painter's `batch_ms` does not include it, and a container style change
    /// spends most of a frame here rather than there.
    flush_ms: Percentiles,
    batch_ms: Percentiles,
    gpu_upload_ms: Percentiles,
}

#[derive(Serialize)]
struct Percentiles {
    p50: f64,
    p95: f64,
    max: f64,
}

impl Percentiles {
    fn of(mut samples: Vec<f64>) -> Self {
        if samples.is_empty() {
            return Self {
                p50: 0.0,
                p95: 0.0,
                max: 0.0,
            };
        }
        samples.sort_by(f64::total_cmp);
        let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
        Self {
            p50: at(0.5),
            p95: at(0.95),
            max: samples[samples.len() - 1],
        }
    }
}

fn main() {
    let mut output = None;
    // Filters, so one cell can be re-measured on an otherwise idle machine
    // rather than inside a fifteen-minute sweep that shares the GPU with
    // whatever else is running.
    let mut only_labels: Vec<usize> = Vec::new();
    let mut only_workload: Vec<String> = Vec::new();
    let mut frame_override: Option<usize> = None;
    let mut depth = 0usize;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => output = args.next(),
            "--labels" => {
                let Some(value) = args.next().and_then(|raw| raw.parse().ok()) else {
                    eprintln!("--labels needs a count");
                    std::process::exit(2);
                };
                only_labels.push(value);
            }
            "--frames" => {
                let Some(value) = args.next().and_then(|raw| raw.parse().ok()) else {
                    eprintln!("--frames needs a count");
                    std::process::exit(2);
                };
                frame_override = Some(value);
            }
            "--depth" => {
                let Some(value) = args.next().and_then(|raw| raw.parse().ok()) else {
                    eprintln!("--depth needs a count");
                    std::process::exit(2);
                };
                depth = value;
            }
            "--workload" => {
                let Some(value) = args.next() else {
                    eprintln!("--workload needs an id");
                    std::process::exit(2);
                };
                only_workload.push(value);
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let Some((device, queue, adapter)) = gpu() else {
        eprintln!("no wgpu adapter; nothing to measure");
        std::process::exit(2);
    };
    let mut cells = Vec::new();
    for labels in LABEL_GRID {
        if !only_labels.is_empty() && !only_labels.contains(&labels) {
            continue;
        }
        for workload in [
            Workload::Static,
            Workload::StaticUnique,
            Workload::Color,
            Workload::Opacity,
            Workload::Transform,
            Workload::Mutate,
        ] {
            if !only_workload.is_empty() && !only_workload.iter().any(|id| id == workload.id()) {
                continue;
            }
            let rates: &[usize] = if workload.animated() {
                &RATE_GRID
            } else {
                &[RATE_GRID[0]]
            };
            for frames in rates {
                let frames = frame_override.unwrap_or(*frames);
                cells.push(run(&device, &queue, workload, labels, frames, depth));
            }
        }
    }
    print_table(&cells);
    let report = Report {
        schema_version: 1,
        adapter,
        cells,
    };
    if let Some(path) = output {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&report).expect("serializable report"),
        )
        .expect("write report");
        println!("\nwrote {path}");
    }
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .ok()?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nana-text-paint-benchmark"),
        ..Default::default()
    }))
    .ok()?;
    Some((device, queue, format!("{} ({:?})", info.name, info.backend)))
}

/// A viewport that holds `labels` boxes of [`LABEL`], roughly square.
fn viewport_for(labels: usize) -> [u32; 2] {
    let area = labels as f32 * LABEL[0] * LABEL[1];
    let width = (area.sqrt().max(LABEL[0]) / LABEL[0]).ceil() * LABEL[0];
    let columns = (width / LABEL[0]).max(1.0);
    let rows = (labels as f32 / columns).ceil().max(1.0);
    [
        (width as u32).clamp(320, 8192),
        ((rows * LABEL[1]) as u32).clamp(240, 8192),
    ]
}

fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    workload: Workload,
    labels: usize,
    frames: usize,
    depth: usize,
) -> Cell {
    let physical = viewport_for(labels);
    let document_id = DocumentId::new(1).expect("document");
    let mut document = RuntimeDocument::new(document_id);
    let root = StableNodeId::new(1).expect("root");
    let column = StableNodeId::new(2).expect("column");
    let ticker = StableNodeId::new(3).expect("ticker");
    let mut build = MutationQueue::new();
    build.create(root, document_id, NodeKind::Document);
    // Wrappers between the document and the labels. A real shell's text sits
    // ten to twenty elements deep, and everything the painter asks per
    // primitive walks that chain.
    let mut parent = root;
    for level in 0..depth {
        let wrapper = StableNodeId::new(1_000_000 + level as u64).expect("wrapper");
        build.create(
            wrapper,
            document_id,
            NodeKind::Element { tag: "div".into() },
        );
        build.insert(parent, wrapper, None);
        build.set_style(wrapper, column_style(None, None));
        parent = wrapper;
    }
    build.create(column, document_id, NodeKind::Element { tag: "div".into() });
    build.insert(parent, column, None);
    build.set_style(column, column_style(None, None));
    build.create(ticker, document_id, NodeKind::Text);
    build.insert(column, ticker, None);
    build.set_text(ticker, TextContent { value: ".".into() });
    build.set_style(ticker, label_style());
    let mut rows = Vec::with_capacity(labels);
    for index in 0..labels {
        let label = StableNodeId::new(4 + index as u64).expect("label");
        build.create(label, document_id, NodeKind::Text);
        build.insert(column, label, None);
        build.set_text(
            label,
            TextContent {
                value: label_text(workload, index),
            },
        );
        build.set_style(label, label_style());
        rows.push(label);
    }
    document
        .context_mut()
        .commit_mutations(build)
        .expect("build document");

    let viewport = LayoutViewport::new(physical[0] as f32, physical[1] as f32);
    let mut shaper = NanaTextShaper::default();
    let mut painter = SceneWgpuPainter::new(device, queue, FORMAT);
    let target = color_target(device, physical);
    let paint_viewport = ScenePaintViewport {
        logical_size: [physical[0] as f32, physical[1] as f32],
        physical_size: physical,
        scale_factor: 1.0,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: [0.08, 0.08, 0.09, 1.0],
        clear: true,
    };

    let mut batch = Vec::new();
    let mut flush = Vec::new();
    let mut upload = Vec::new();
    let mut warm = None;
    let mut warm_glyph = None;
    let mut warm_shape = None;
    for frame in 0..WARMUP_FRAMES + frames {
        mutate(&mut document, workload, column, ticker, &rows, frame);
        let flush_started = std::time::Instant::now();
        document.flush(viewport, &mut shaper).expect("flush");
        let flush_elapsed = flush_started.elapsed();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-text-paint-benchmark"),
        });
        painter
            .paint(
                document.scene(),
                &mut encoder,
                &target,
                paint_viewport,
                None,
                None,
            )
            .expect("paint");
        queue.submit([encoder.finish()]);
        if frame + 1 == WARMUP_FRAMES {
            warm = Some(painter.text_glyph_counters());
            warm_glyph = Some(painter.text_glyph_counters().text_gpu_entry_glyphs);
            warm_shape = Some(painter.text_shape_cache_stats());
        }
        if frame >= WARMUP_FRAMES {
            flush.push(flush_elapsed.as_secs_f64() * 1000.0);
            let timings = painter.last_gpu_timings().expect("timed frame");
            batch.push(timings.batch.as_secs_f64() * 1000.0);
            upload.push(timings.gpu_upload.as_secs_f64() * 1000.0);
        }
    }
    let warm = warm.expect("warm counters");
    let end = painter.text_glyph_counters();
    let per_frame = frames.max(1) as f64;
    let mut counters = BTreeMap::new();
    let mut delta = |name: &str, after: u64, before: u64| {
        counters.insert(
            name.to_string(),
            (after.saturating_sub(before)) as f64 / per_frame,
        );
    };
    delta(
        "glyph_resolve_requests",
        end.glyph_resolve_requests,
        warm.glyph_resolve_requests,
    );
    delta(
        "glyph_rasterized",
        end.glyph_rasterized,
        warm.glyph_rasterized,
    );
    delta(
        "glyph_upload_bytes",
        end.glyph_upload_bytes,
        warm.glyph_upload_bytes,
    );
    delta(
        "text_instance_rebuilds",
        end.text_instance_rebuilds,
        warm.text_instance_rebuilds,
    );
    delta(
        "text_instance_patches",
        end.text_instance_patches,
        warm.text_instance_patches,
    );
    delta(
        "text_instance_upload_bytes",
        end.text_instance_upload_bytes,
        warm.text_instance_upload_bytes,
    );
    delta(
        "text_presentation_upload_bytes",
        end.text_presentation_upload_bytes,
        warm.text_presentation_upload_bytes,
    );
    delta(
        "text_prepare_nodes_considered",
        end.text_prepare_nodes_considered,
        warm.text_prepare_nodes_considered,
    );
    delta(
        "text_prepare_nodes_skipped",
        end.text_prepare_nodes_skipped,
        warm.text_prepare_nodes_skipped,
    );
    delta(
        "text_prepare_nodes_culled",
        end.text_prepare_nodes_culled,
        warm.text_prepare_nodes_culled,
    );
    counters.insert(
        "text_gpu_entries_active".to_string(),
        end.text_gpu_entries_active as f64,
    );
    let (_, warm_misses, warm_evictions) = warm_shape.unwrap_or_default();
    let (_, misses, evictions) = painter.text_shape_cache_stats();
    counters.insert(
        "shape_cache_misses".to_string(),
        (misses - warm_misses) as f64 / per_frame,
    );
    counters.insert(
        "shape_cache_evictions".to_string(),
        (evictions - warm_evictions) as f64 / per_frame,
    );
    Cell {
        workload: workload.id().to_string(),
        labels,
        depth,
        viewport: physical,
        frames,
        live_glyphs: warm_glyph.unwrap_or_default(),
        counters,
        flush_ms: Percentiles::of(flush),
        batch_ms: Percentiles::of(batch),
        gpu_upload_ms: Percentiles::of(upload),
    }
}

/// Change exactly one thing, so the cell's counters name that one thing.
fn mutate(
    document: &mut RuntimeDocument,
    workload: Workload,
    column: StableNodeId,
    ticker: StableNodeId,
    rows: &[StableNodeId],
    frame: usize,
) {
    let mut queue = MutationQueue::new();
    match workload {
        Workload::Static | Workload::StaticUnique => {
            // Not a text change *to the labels*: the label beside an animation
            // is what a shell spends its frames on.
            queue.set_text(
                ticker,
                TextContent {
                    value: ".".repeat(1 + frame % 3),
                },
            );
        }
        Workload::Color => {
            let role = if frame.is_multiple_of(2) {
                SemanticColorRole::Text
            } else {
                SemanticColorRole::Muted
            };
            for row in rows {
                queue.set_style(
                    *row,
                    NodeStyle {
                        foreground: Some(role),
                        ..NodeStyle::default()
                    },
                );
            }
        }
        Workload::Opacity => {
            let opacity = 0.35 + 0.6 * ((frame % 32) as f32 / 32.0);
            queue.set_style(column, column_style(Some(opacity), None));
        }
        Workload::Transform => {
            let angle = (frame % 360) as f32 * std::f32::consts::PI / 180.0;
            let (sin, cos) = angle.sin_cos();
            queue.set_style(
                column,
                column_style(
                    None,
                    Some(PaintTransform {
                        a: cos,
                        b: sin,
                        c: -sin,
                        d: cos,
                        ..PaintTransform::default()
                    }),
                ),
            );
        }
        Workload::Mutate => {
            // One row in a hundred, not one in `len / 100`: the first reading
            // of that is "every row" as soon as the list is short.
            for row in rows.iter().step_by(100) {
                queue.set_text(
                    *row,
                    TextContent {
                        value: format!("Tick {frame}"),
                    },
                );
            }
        }
    }
    document
        .context_mut()
        .commit_mutations(queue)
        .expect("frame mutation");
}

/// What one label says. Repeated by default; see [`Workload::StaticUnique`].
fn label_text(workload: Workload, index: usize) -> String {
    match workload {
        Workload::StaticUnique => format!("Row {index}"),
        _ => format!("Row {}", index % DISTINCT_LABELS),
    }
}

fn label_style() -> NodeStyle {
    NodeStyle {
        layout: Arc::new(LayoutStyle {
            width: Some(LengthSpec::Px(LABEL[0])),
            height: Some(LengthSpec::Px(LABEL[1])),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    }
}

fn column_style(opacity: Option<f32>, transform: Option<PaintTransform>) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Fill),
            direction: Some(FlexDirection::Row),
            flex_wrap: FlexWrap::Wrap,
            opacity,
            transform,
            transform_origin: transform.is_some().then(TransformOrigin::default),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    }
}

fn color_target(device: &wgpu::Device, physical: [u32; 2]) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-text-paint-benchmark target"),
            size: wgpu::Extent3d {
                width: physical[0],
                height: physical[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn print_table(cells: &[Cell]) {
    println!(
        "{:<12} {:>7} {:>6} {:>8} {:>10} {:>10} {:>12} {:>10} {:>10} {:>10}",
        "workload",
        "labels",
        "Hz",
        "glyphs",
        "resolve/f",
        "rebuild/f",
        "inst B/f",
        "reshape/f",
        "flush p50",
        "batch p50"
    );
    for cell in cells {
        let get = |name: &str| cell.counters.get(name).copied().unwrap_or_default();
        println!(
            "{:<12} {:>7} {:>6} {:>8} {:>10.1} {:>10.1} {:>12.0} {:>10.0} {:>9.3}m {:>9.3}m",
            cell.workload,
            cell.labels,
            cell.frames,
            cell.live_glyphs,
            get("glyph_resolve_requests"),
            get("text_instance_rebuilds"),
            get("text_instance_upload_bytes"),
            get("shape_cache_misses"),
            cell.flush_ms.p50,
            cell.batch_ms.p50,
        );
    }
}
