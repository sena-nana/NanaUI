//! Headless GPU scene bench for Issue #8 `gpu-scene-*`.
//!
//! Loads `perf/scenarios/gpu-scene-*.json`. UiOnly materializes that file's
//! viewport, host-texture slot, and UI nodes, then paints through
//! `SceneWgpuPainter`. No CPU readback. Missing adapter or Live2D exit 2.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_ui::runtime::{
    Button, DocumentId, FrameProfile, FrameProfiler, GpuTextureView, GpuWorkObservation,
    HOST_TEXTURE_RENDERER, LayoutStyle, LayoutViewport, LengthSpec, List, NodeStyle,
    RuntimeDocument, StageStatus, Text,
};
use nana_ui::{
    ButtonKind, GpuStageTimings, HostTexture, HostTextureAlphaMode, HostTextureRegistry,
    NanaTextShaper, ScenePaintViewport, SceneWgpuPainter,
};
use nana_ui_scene::ScenePrimitiveKind;
use serde::{Deserialize, Serialize};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const WARMUP: usize = 3;
const FRAMES: usize = 20;

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    unsupported_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scenario_id: Option<String>,
    composition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    materialization: Option<Materialization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frames: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gpu_work: Option<GpuWorkSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_stages: Option<BTreeMap<String, StageStatusReport>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stages: Option<StageReport>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
struct HostTextureParams {
    slot: String,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
struct Materialization {
    viewport: [u32; 2],
    host_texture: HostTextureParams,
    ui_nodes: Vec<String>,
    ui_entity_count: usize,
    scene_primitive_kinds: Vec<String>,
}

#[derive(Serialize, Clone, Copy)]
struct GpuWorkSnapshot {
    batch_rebuilds: usize,
    draw_batches: usize,
    draw_calls: usize,
    gpu_upload_bytes: usize,
    gpu_buffer_reallocations: usize,
}

impl From<GpuWorkObservation> for GpuWorkSnapshot {
    fn from(observed: GpuWorkObservation) -> Self {
        Self {
            batch_rebuilds: observed.batch_rebuilds,
            draw_batches: observed.draw_batches,
            draw_calls: observed.draw_calls,
            gpu_upload_bytes: observed.gpu_upload_bytes,
            gpu_buffer_reallocations: observed.gpu_buffer_reallocations,
        }
    }
}

#[derive(Serialize)]
struct StageStatusReport {
    status: &'static str,
}

#[derive(Serialize)]
struct StageReport {
    batch_ms: Distribution,
    gpu_upload_ms: Distribution,
    encode_ms: Distribution,
    submit_ms: Distribution,
}

#[derive(Serialize)]
struct Distribution {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

#[derive(Deserialize)]
struct ScenarioFile {
    id: String,
    kind: String,
    params: ScenarioParams,
}

#[derive(Deserialize)]
struct ScenarioParams {
    composition: String,
    viewport: [u32; 2],
    host_texture: HostTextureParams,
    ui_nodes: Vec<String>,
}

fn main() {
    let args = Args::parse();
    let report = match load_scenario(args.scenario.as_ref()) {
        Ok(scenario) => run_scenario(scenario),
        Err(error) => unsupported(error.scenario_id, &error.composition, error.reason),
    };
    write_report(&args.output, &report);
    if report.status != "ok" {
        std::process::exit(2);
    }
}

struct LoadError {
    scenario_id: Option<String>,
    composition: String,
    reason: String,
}

fn load_err(scenario_id: Option<String>, composition: &str, reason: String) -> LoadError {
    LoadError {
        scenario_id,
        composition: composition.to_string(),
        reason,
    }
}

struct Args {
    output: Option<PathBuf>,
    scenario: Option<PathBuf>,
}

impl Args {
    fn parse() -> Self {
        let mut output = None;
        let mut scenario = None;
        let mut argv = std::env::args().skip(1);
        while let Some(arg) = argv.next() {
            match arg.as_str() {
                "--output" => output = argv.next().map(PathBuf::from),
                "--scenario" => scenario = argv.next().map(PathBuf::from),
                _ => {}
            }
        }
        Self { output, scenario }
    }
}

fn load_scenario(path: Option<&PathBuf>) -> Result<ScenarioFile, LoadError> {
    let Some(path) = path else {
        return Err(load_err(
            None,
            "unknown",
            "gpu-scene-* must load perf/scenarios/gpu-scene-*.json via --scenario. \
Do not invent a private hosted-gpu-demo tree."
                .into(),
        ));
    };
    let text = fs::read_to_string(path).map_err(|error| {
        load_err(
            None,
            "unknown",
            format!("failed to read scenario JSON {}: {error}", path.display()),
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        load_err(
            None,
            "unknown",
            format!("invalid scenario JSON {}: {error}", path.display()),
        )
    })?;
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let composition = value
        .pointer("/params/composition")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    if composition != "UiOnly" {
        return Err(load_err(
            Some(id),
            &composition,
            live2d_reason(&composition),
        ));
    }
    serde_json::from_value(value).map_err(|error| {
        load_err(
            Some(id),
            &composition,
            format!(
                "gpu-scene-ui UiOnly requires viewport, host_texture, and ui_nodes from the scenario JSON: {error}"
            ),
        )
    })
}

fn live2d_reason(composition: &str) -> String {
    format!(
        "GpuScene composition {composition} needs a real Live2D Scene pass. \
HostTexture evidence from a UiOnly encode is not Live2D. Required by #8 / not implemented."
    )
}

fn unsupported(scenario_id: Option<String>, composition: &str, reason: String) -> Report {
    Report {
        schema_version: 1,
        status: "unsupported",
        unsupported_reason: Some(reason),
        scenario_id,
        composition: composition.to_string(),
        materialization: None,
        adapter: None,
        frames: None,
        gpu_work: None,
        frame_stages: None,
        stages: None,
    }
}

fn run_scenario(scenario: ScenarioFile) -> Report {
    if scenario.kind != "GpuScene" || scenario.params.composition != "UiOnly" {
        return unsupported(
            Some(scenario.id),
            &scenario.params.composition,
            live2d_reason(&scenario.params.composition),
        );
    }
    run_ui_only(scenario)
}

fn run_ui_only(scenario: ScenarioFile) -> Report {
    let params = &scenario.params;
    let Some((device, queue, adapter)) = request_device() else {
        return unsupported(
            Some(scenario.id),
            "UiOnly",
            "No WGPU adapter for the hosted GPU scene path. Do not invent upload/batch zeros."
                .into(),
        );
    };
    let slot = params.host_texture.slot.as_str();
    let preview = HostSlotContent::new(
        &device,
        &queue,
        (params.host_texture.width, params.host_texture.height),
    );
    let textures = HostTextureRegistry::new();
    textures.register(
        slot,
        preview.texture(),
        params.host_texture.width,
        params.host_texture.height,
        HostTextureAlphaMode::Premultiplied,
    );

    let mut document = match ui_document(params) {
        Ok(document) => document,
        Err(reason) => return unsupported(Some(scenario.id), "UiOnly", reason),
    };
    let mut shaper = NanaTextShaper::default();
    let viewport = LayoutViewport::new(params.viewport[0] as f32, params.viewport[1] as f32);
    document
        .flush(viewport, &mut shaper)
        .expect("gpu-scene-ui flush");

    let materialization = Materialization {
        viewport: params.viewport,
        host_texture: params.host_texture.clone(),
        ui_nodes: params.ui_nodes.clone(),
        ui_entity_count: document
            .context()
            .world()
            .last_work_counters()
            .entities_total,
        scene_primitive_kinds: scene_primitive_kinds(document.scene(), slot),
    };

    let mut painter = SceneWgpuPainter::new(&device, &queue, FORMAT);
    let target = color_target(&device, params.viewport[0], params.viewport[1]);
    let paint_viewport = ScenePaintViewport {
        logical_size: [params.viewport[0] as f32, params.viewport[1] as f32],
        physical_size: params.viewport,
        scale_factor: 1.0,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: [0.08, 0.08, 0.09, 1.0],
        clear: true,
    };

    let mut batch = Vec::with_capacity(FRAMES);
    let mut upload = Vec::with_capacity(FRAMES);
    let mut encode = Vec::with_capacity(FRAMES);
    let mut submit = Vec::with_capacity(FRAMES);
    let mut last_work = None;
    let mut last_stages = None;
    for frame in 0..(WARMUP + FRAMES) {
        preview.render(&device, &queue, frame as u32);
        textures.register(
            slot,
            preview.texture(),
            params.host_texture.width,
            params.host_texture.height,
            HostTextureAlphaMode::Premultiplied,
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-gpu-scene-benchmark"),
        });
        painter
            .paint(
                document.scene(),
                &mut encoder,
                &target,
                paint_viewport,
                Some(&textures),
                None,
            )
            .expect("gpu-scene-ui paint");
        let submit_started = Instant::now();
        queue.submit([encoder.finish()]);
        let submit_elapsed = submit_started.elapsed();
        painter.record_submit(submit_elapsed);
        let timings = painter
            .last_gpu_timings()
            .expect("encoded GPU scene frame must time stages");
        let work = painter
            .last_gpu_work()
            .expect("encoded GPU scene frame must record counters");
        let frame_stages = host_frame_stages(document.context().last_frame_profile(), timings);
        if frame >= WARMUP {
            batch.push(timings.batch);
            upload.push(timings.gpu_upload);
            encode.push(timings.encode);
            submit.push(timings.submit);
            last_work = Some(work);
            last_stages = Some(frame_stages);
        }
    }

    let work = last_work.expect("warmup completed");
    Report {
        schema_version: 1,
        status: "ok",
        unsupported_reason: None,
        scenario_id: Some(scenario.id),
        composition: "UiOnly".into(),
        materialization: Some(materialization),
        adapter: Some(adapter),
        frames: Some(FRAMES),
        gpu_work: Some(GpuWorkSnapshot::from(work)),
        frame_stages: last_stages,
        stages: Some(StageReport {
            batch_ms: summarize(&batch),
            gpu_upload_ms: summarize(&upload),
            encode_ms: summarize(&encode),
            submit_ms: summarize(&submit),
        }),
    }
}

fn ui_document(params: &ScenarioParams) -> Result<RuntimeDocument, String> {
    if !params.ui_nodes.iter().any(|node| node == "list") {
        return Err("UiOnly ui_nodes must include list as the document root".into());
    }
    if !params
        .ui_nodes
        .iter()
        .any(|node| node == "gpu-texture-view")
    {
        return Err(
            "UiOnly ui_nodes must include gpu-texture-view for the GPU content slot".into(),
        );
    }
    let document_id = DocumentId::new(1).expect("gpu-scene document");
    let mut document = RuntimeDocument::new(document_id);
    let root = document
        .context_mut()
        .create_component(document_id, List::new().label("gpu-scene-ui"))
        .expect("list");
    for kind in &params.ui_nodes {
        match kind.as_str() {
            "list" => {}
            "text" => {
                let child = document
                    .context_mut()
                    .create_component(document_id, Text::new("UiOnly"))
                    .expect("text");
                document
                    .context_mut()
                    .append_child(root, child)
                    .expect("text child");
            }
            "gpu-texture-view" => {
                let child = document
                    .context_mut()
                    .create_component(
                        document_id,
                        GpuTextureView::new(params.host_texture.slot.as_str()).style(slot_style(
                            params.host_texture.width,
                            params.host_texture.height,
                        )),
                    )
                    .expect("gpu-texture-view");
                document
                    .context_mut()
                    .append_child(root, child)
                    .expect("slot child");
            }
            "button" => {
                let child = document
                    .context_mut()
                    .create_component(
                        document_id,
                        Button::new("HostTexture").kind(ButtonKind::Primary),
                    )
                    .expect("button");
                document
                    .context_mut()
                    .append_child(root, child)
                    .expect("button child");
            }
            other => {
                return Err(format!(
                    "UiOnly ui_nodes contains unknown node {other}; catalog allows list/text/gpu-texture-view/button"
                ));
            }
        }
    }
    Ok(document)
}

fn slot_style(width: u32, height: u32) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    *layout = LayoutStyle {
        width: Some(LengthSpec::Px(width as f32)),
        height: Some(LengthSpec::Px(height as f32)),
        ..LayoutStyle::default()
    };
    style
}

fn scene_primitive_kinds(scene: &nana_ui_scene::UiScene, slot: &str) -> Vec<String> {
    let mut kinds = Vec::new();
    for primitive in scene.primitives() {
        let name = primitive_kind_name(&primitive.kind, slot);
        if !kinds.iter().any(|existing| existing == name) {
            kinds.push(name.to_string());
        }
    }
    kinds.sort();
    kinds
}

fn primitive_kind_name(kind: &ScenePrimitiveKind, slot: &str) -> &'static str {
    match kind {
        ScenePrimitiveKind::Quad { .. }
        | ScenePrimitiveKind::QuadBatch { .. }
        | ScenePrimitiveKind::QuadColorBatch { .. } => "quad",
        ScenePrimitiveKind::Text { .. } => "text",
        ScenePrimitiveKind::Icon { .. } | ScenePrimitiveKind::IconBatch { .. } => "icon",
        ScenePrimitiveKind::Spinner { .. } => "spinner",
        ScenePrimitiveKind::Stroke { .. } => "stroke",
        ScenePrimitiveKind::Custom { node: custom, .. }
            if custom.renderer.as_ref() == HOST_TEXTURE_RENDERER
                && custom.resource.as_ref() == slot =>
        {
            "host-texture"
        }
        ScenePrimitiveKind::Custom { .. } => "custom",
    }
}

fn host_frame_stages(
    cpu: &FrameProfile,
    timings: GpuStageTimings,
) -> BTreeMap<String, StageStatusReport> {
    let mut profiler = FrameProfiler::new();
    for timing in &cpu.stages {
        if timing.stage.gpu_host_owned() {
            continue;
        }
        match timing.status {
            StageStatus::Ran => profiler.record(timing.stage, timing.duration),
            StageStatus::Skipped => profiler.skip(timing.stage),
            StageStatus::Unsupported => profiler.unsupported(timing.stage),
        }
    }
    timings.record_on(&mut profiler);
    profiler
        .finish()
        .stages
        .into_iter()
        .map(|timing| {
            (
                format!("{:?}", timing.stage),
                StageStatusReport {
                    status: stage_status_name(timing.status),
                },
            )
        })
        .collect()
}

fn stage_status_name(status: StageStatus) -> &'static str {
    match status {
        StageStatus::Ran => "ran",
        StageStatus::Skipped => "skipped",
        StageStatus::Unsupported => "unsupported",
    }
}

fn request_device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .ok()?;
    let info = adapter.get_info();
    let label = format!("{} ({:?})", info.name, info.backend);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nana-gpu-scene-benchmark"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .ok()?;
    Some((device, queue, label))
}

fn color_target(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-gpu-scene-benchmark target"),
            size: wgpu::Extent3d {
                width,
                height,
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

struct HostSlotContent {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    host: HostTexture,
}

impl HostSlotContent {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, size: (u32, u32)) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nana-gpu-scene-benchmark slot"),
            source: wgpu::ShaderSource::Wgsl(SLOT_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-gpu-scene-benchmark slot layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nana-gpu-scene-benchmark slot pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nana-gpu-scene-benchmark slot pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nana-gpu-scene-benchmark slot uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nana-gpu-scene-benchmark slot bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-gpu-scene-benchmark host texture"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let host = HostTexture::from_wgpu(1, 1, view.clone());
        Self {
            pipeline,
            bind_group,
            uniform,
            _texture: texture,
            view,
            host,
        }
    }

    fn texture(&self) -> HostTexture {
        self.host.clone()
    }

    fn write_uniform(&self, queue: &wgpu::Queue, frame: u32) {
        let seed = frame as f32 * 0.17;
        let parameters = [seed, 0.0, 0.0, 0.0];
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&parameters));
        self.host.invalidate();
    }

    fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, frame: u32) {
        self.write_uniform(queue, frame);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-gpu-scene-benchmark host slot"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nana-gpu-scene-benchmark host slot"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}

const SLOT_SHADER: &str = r#"
struct SceneUniform {
    parameters: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> scene: SceneUniform;

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[index], 0.0, 1.0);
}

@fragment
fn fragment_main() -> @location(0) vec4<f32> {
    let seed = scene.parameters.x;
    return vec4<f32>(0.12 + seed * 0.01, 0.28, 0.46, 1.0);
}
"#;

fn summarize(samples: &[Duration]) -> Distribution {
    let mut values = samples
        .iter()
        .map(|sample| sample.as_secs_f64() * 1_000.0)
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.partial_cmp(right).unwrap());
    Distribution {
        p50: percentile(&values, 0.50),
        p95: percentile(&values, 0.95),
        p99: percentile(&values, 0.99),
        max: values.last().copied().unwrap_or(0.0),
    }
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * quantile).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn write_report(path: &Option<PathBuf>, report: &Report) {
    let json = serde_json::to_string_pretty(report).expect("serialize gpu-scene report") + "\n";
    if let Some(path) = path {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).expect("write gpu-scene report directory");
        }
        fs::write(path, json).expect("write gpu-scene report");
    } else {
        print!("{json}");
    }
}
