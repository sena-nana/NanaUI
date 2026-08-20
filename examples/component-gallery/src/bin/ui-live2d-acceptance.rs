//! macOS acceptance for the supported Live2D -> host texture -> NanaUI path.
//!
//! This binary intentionally lives outside NanaUI's public API. Live2D owns
//! model evaluation and rendering; NanaUI only samples host-owned textures as
//! ordinary `CustomRenderNode` slots. This harness composes two HostTexture
//! slots with in-card Selected Button chrome between them.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use live2d_core::{
    BlendColor, ClippingInfo, DrawableId, FrameDirtyFlags, ModelDynamicFrame, ModelGeometryFrame,
    ModelStaticData, RenderObject, RuntimeFrame, TextureAsset, Vertex,
};
use live2d_test_support::{DrawableBuilder, SnapshotBuilder};
use live2d_wgpu::{
    RegistrationRequest, RenderTarget, RenderView, Renderer as Live2dRenderer, RendererOptions,
    SubmissionBatch, SubmissionToken,
};
use nana_ui::runtime::{
    Button as RuntimeButton, Card as RuntimeCard, CustomRenderNode, DocumentId, GpuTextureView,
    LayoutBox, MutationQueue, NodeKind, RuntimeDocument, Text as RuntimeText, UiScene,
};
use nana_ui::{
    ButtonKind, HostTexture, HostTextureAlphaMode, HostTextureRegistry, NanaTextShaper,
    ScenePaintViewport, SceneResourceEncodeContext, SceneResourceProducer,
    SceneResourceProducerRegistry, SceneWgpuPainter, ThemeMode, ThemeModeExt,
};
use serde::Serialize;

#[path = "ui_snapshots/render/offscreen.rs"]
mod offscreen;
#[path = "ui_snapshots/write.rs"]
mod write;

use write::Size;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;
const LIVE2D_SIZE: u32 = 512;
const PREVIEW_X: f32 = 194.0;
const PREVIEW_Y: f32 = 64.0;
const PREVIEW_SIZE: f32 = 512.0;
const FG_BAND_HEIGHT: f32 = 120.0;
const CHROME_BUTTON_X: f32 = 370.0;
const CHROME_BUTTON_Y: f32 = 302.0;
const CHROME_BUTTON_WIDTH: f32 = 160.0;
const CHROME_BUTTON_HEIGHT: f32 = 36.0;
/// Left of the centered "Start" label so the sample hits the opaque fill.
const CHROME_FILL_SAMPLE_X: u32 = (CHROME_BUTTON_X + 12.0) as u32;
const CHROME_FILL_SAMPLE_Y: u32 = (CHROME_BUTTON_Y + CHROME_BUTTON_HEIGHT / 2.0) as u32;
const BG_FILL: wgpu::Color = wgpu::Color {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};
const BG_FILL_RGBA: [u8; 4] = [255, 0, 255, 255];
const CHROME_FILL_CHANNEL_SLACK: i16 = 24;
const WARMUP: usize = 20;
const ITERATIONS: usize = 80;
const LIVE2D_REVISION: &str = "71e92d04ab1b377aae6dac66d6f1ec5f9bb6d033";

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct Sample {
    cpu_ms: f64,
    submit_to_complete_ms: f64,
    total_ms: f64,
}

#[derive(Debug, Serialize)]
struct Distribution {
    p50: Sample,
    p95: Sample,
    p99: Sample,
    max: Sample,
    frame_budget_ms: f64,
    frame_budget_misses: usize,
}

#[derive(Debug, Serialize)]
struct Report {
    platform: &'static str,
    adapter: String,
    backend: String,
    live2d_revision: &'static str,
    workload: &'static str,
    viewport: [u32; 2],
    live2d_target: [u32; 2],
    warmup_iterations: usize,
    measured_iterations: usize,
    ui_only: Distribution,
    live2d_only: Distribution,
    ui_live2d_composed: Distribution,
    screenshot: String,
    screenshot_checksum: u64,
    screenshot_distinct_colors: usize,
}

#[derive(Debug, Clone, Copy)]
enum Workload {
    Ui,
    Live2d,
    Composed,
}

struct LayerTextures {
    background: HostTexture,
    foreground: HostTexture,
}

struct PendingLive2dSubmission {
    token: SubmissionToken,
    total_started: Instant,
    cpu_ms: f64,
}

struct Live2dProducerState {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    renderer: Live2dRenderer,
    handle: live2d_wgpu::ModelHandle,
    static_model: ModelStaticData,
    dynamic: ModelDynamicFrame,
    geometry: ModelGeometryFrame,
    sequence: usize,
    pending: Option<PendingLive2dSubmission>,
}

struct Live2dSceneProducer {
    state: Mutex<Live2dProducerState>,
    host_texture: HostTexture,
}

impl fmt::Debug for Live2dSceneProducer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Live2dSceneProducer").finish()
    }
}

impl Live2dSceneProducer {
    fn complete(&self, device: &wgpu::Device, submission: wgpu::SubmissionIndex) -> Sample {
        let wait_started = Instant::now();
        wait_gpu(
            device,
            submission,
            "Scene-managed Live2D GPU frame completes",
        );
        self.finish_pending(wait_started.elapsed())
    }

    fn finish_pending(&self, submit_to_complete: Duration) -> Sample {
        let mut state = self.state.lock().expect("Live2D producer state");
        let pending = state.pending.take().expect("submitted Live2D frame");
        state
            .renderer
            .complete_submission(
                SubmissionBatch::from_token(pending.token),
                submit_to_complete,
            )
            .expect("release Scene-managed Live2D submission");
        self.host_texture.invalidate();
        Sample {
            cpu_ms: pending.cpu_ms,
            submit_to_complete_ms: duration_ms(submit_to_complete),
            total_ms: elapsed_ms(pending.total_started),
        }
    }
}

impl SceneResourceProducer for Live2dSceneProducer {
    fn encode(
        &self,
        _node: &CustomRenderNode,
        context: SceneResourceEncodeContext<'_>,
    ) -> Result<(), String> {
        let started = Instant::now();
        let mut state = self.state.lock().map_err(|_| "producer lock poisoned")?;
        if state.pending.is_some() {
            return Err("previous Live2D submission is still pending".into());
        }
        let Live2dProducerState {
            texture,
            view,
            renderer,
            handle,
            static_model,
            dynamic,
            geometry,
            sequence,
            pending,
        } = &mut *state;
        let drawable_index = *sequence % dynamic.drawables.len();
        dynamic.drawables[drawable_index].opacity = 0.72 + (*sequence % 8) as f32 * 0.035;
        let frame = RuntimeFrame {
            sequence: *sequence as u64 + 1,
            static_model,
            dynamic,
            geometry,
            dirty: FrameDirtyFlags {
                dynamic: true,
                opacity: true,
                ..FrameDirtyFlags::default()
            },
        };
        renderer
            .update_model(context.queue, *handle, &frame)
            .map_err(|error| error.to_string())?;
        let prepared = renderer
            .prepare_model(
                context.device,
                context.queue,
                context.encoder,
                *handle,
                RenderView::full_target(
                    [LIVE2D_SIZE, LIVE2D_SIZE],
                    glam::Mat4::IDENTITY,
                    glam::Mat4::IDENTITY,
                ),
            )
            .map_err(|error| error.to_string())?;
        let encoded = renderer
            .encode_model(
                context.device,
                context.queue,
                context.encoder,
                RenderTarget::clear(texture, view, wgpu::Color::TRANSPARENT),
                &prepared,
            )
            .map_err(|error| error.to_string())?;
        *sequence += 1;
        *pending = Some(PendingLive2dSubmission {
            token: encoded.submission,
            total_started: started,
            cpu_ms: elapsed_ms(started),
        });
        Ok(())
    }
}

fn main() {
    assert_eq!(
        std::env::consts::OS,
        "macos",
        "this acceptance currently targets macOS"
    );
    let (report_path, screenshot_path) = arguments();
    let report = run(&screenshot_path);
    let json = serde_json::to_string_pretty(&report).expect("serialize acceptance report");
    if let Some(parent) = report_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).expect("create report directory");
    }
    std::fs::write(&report_path, format!("{json}\n")).expect("write acceptance report");
    println!("{}", report_path.display());
}

fn arguments() -> (PathBuf, PathBuf) {
    let mut args = std::env::args_os().skip(1);
    let mut report = None;
    let mut screenshot = None;
    while let Some(flag) = args.next() {
        match flag.to_string_lossy().as_ref() {
            "--output" => report = Some(PathBuf::from(args.next().expect("--output path"))),
            "--screenshot" => {
                screenshot = Some(PathBuf::from(args.next().expect("--screenshot path")))
            }
            other => panic!("unsupported argument `{other}`"),
        }
    }
    (
        report.unwrap_or_else(|| PathBuf::from("ui-live2d-acceptance.json")),
        screenshot.unwrap_or_else(|| PathBuf::from("ui-live2d-acceptance.png")),
    )
}

fn run(screenshot_path: &Path) -> Report {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("acceptance requires a WGPU adapter");
    let adapter_info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("NanaUI Live2D acceptance device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .expect("acceptance requires a WGPU device");

    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let mut painter = SceneWgpuPainter::new(&device, &queue, format);
    let ui_target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("NanaUI acceptance target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let ui_target_view = ui_target.create_view(&wgpu::TextureViewDescriptor::default());

    let live2d_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Live2D host texture"),
        size: wgpu::Extent3d {
            width: LIVE2D_SIZE,
            height: LIVE2D_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let live2d_view = live2d_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let host_texture = HostTexture::from_wgpu(7, 1, live2d_view.clone());
    let (_background_keep, background_view) =
        solid_host_texture(&device, &queue, format, LIVE2D_SIZE, LIVE2D_SIZE, BG_FILL);
    let background_texture = HostTexture::from_wgpu(8, 1, background_view);
    let layers = LayerTextures {
        background: background_texture,
        foreground: host_texture.clone(),
    };

    let snapshot = synthetic_model();
    let static_model = ModelStaticData::from_snapshot(&snapshot);
    let dynamic = ModelDynamicFrame::from_snapshot(&snapshot);
    let geometry = ModelGeometryFrame::from_snapshot(&snapshot);
    let mut live2d_renderer =
        Live2dRenderer::new(&device, RendererOptions::sdr(format)).expect("create Live2D renderer");
    let handle = live2d_renderer
        .register(&device, &queue, RegistrationRequest::new(&static_model))
        .expect("register synthetic acceptance model")
        .handle;
    let producer = Arc::new(Live2dSceneProducer {
        state: Mutex::new(Live2dProducerState {
            texture: live2d_texture,
            view: live2d_view,
            renderer: live2d_renderer,
            handle,
            static_model,
            dynamic,
            geometry,
            sequence: 0,
            pending: None,
        }),
        host_texture: host_texture.clone(),
    });
    let mut resource_producers = SceneResourceProducerRegistry::new();
    resource_producers.insert("live2d", producer.clone());
    let resource_scene = live2d_resource_scene(host_texture.version());

    let ui_only_scene = acceptance_scene(None);
    let composed_scene = acceptance_scene(Some(&layers));
    let mut ui_only = Vec::with_capacity(ITERATIONS);
    let mut live2d_only = Vec::with_capacity(ITERATIONS);
    let mut composed = Vec::with_capacity(ITERATIONS);

    for iteration in 0..(WARMUP + ITERATIONS) {
        let order = match iteration % 3 {
            0 => [Workload::Live2d, Workload::Composed, Workload::Ui],
            1 => [Workload::Composed, Workload::Ui, Workload::Live2d],
            _ => [Workload::Ui, Workload::Live2d, Workload::Composed],
        };
        let mut ui_sample = None;
        let mut live2d_sample = None;
        let mut composed_sample = None;
        for workload in order {
            match workload {
                Workload::Ui => {
                    ui_sample = Some(
                        paint_scene(
                            &mut painter,
                            &ui_only_scene.0,
                            None,
                            &device,
                            &queue,
                            &ui_target_view,
                        )
                        .0,
                    );
                }
                Workload::Live2d => {
                    let submission = resource_producers
                        .encode_scene(&resource_scene, &device, &queue)
                        .expect("encode graph-managed Live2D resource")
                        .expect("Live2D resource producer is registered");
                    live2d_sample = Some(producer.complete(&device, submission));
                }
                Workload::Composed => {
                    let total_started = Instant::now();
                    resource_producers
                        .encode_scene(&resource_scene, &device, &queue)
                        .expect("encode graph-managed composed Live2D resource")
                        .expect("Live2D resource producer is registered");
                    let ui = submit_scene(
                        &mut painter,
                        &composed_scene.0,
                        Some(&composed_scene.1),
                        &device,
                        &queue,
                        &ui_target_view,
                    );
                    let wait_started = Instant::now();
                    wait_gpu(&device, ui.submission, "composed UI GPU frame completes");
                    let fence = wait_started.elapsed();
                    let live2d = producer.finish_pending(fence);
                    composed_sample = Some(Sample {
                        cpu_ms: live2d.cpu_ms + ui.cpu_ms,
                        submit_to_complete_ms: duration_ms(fence),
                        total_ms: elapsed_ms(total_started),
                    });
                }
            }
        }
        if iteration >= WARMUP {
            ui_only.push(ui_sample.expect("UI workload sampled"));
            live2d_only.push(live2d_sample.expect("Live2D workload sampled"));
            composed.push(composed_sample.expect("composed workload sampled"));
        }
    }

    resource_producers
        .encode_scene(&resource_scene, &device, &queue)
        .expect("encode final graph-managed Live2D resource")
        .expect("Live2D resource producer is registered");
    let ui = submit_scene(
        &mut painter,
        &composed_scene.0,
        Some(&composed_scene.1),
        &device,
        &queue,
        &ui_target_view,
    );
    wait_gpu(
        &device,
        ui.submission,
        "final composed UI GPU frame completes",
    );
    let _ = producer.finish_pending(Duration::ZERO);
    let pixels = screenshot_target(&device, &queue, &ui_target);
    write::png(screenshot_path, Size::new(WIDTH, HEIGHT), &pixels).expect("write screenshot");
    let distinct_colors = distinct_colors(&pixels);
    assert!(
        distinct_colors > 32,
        "composed screenshot must contain rendered detail"
    );
    assert_chrome_between_host_texture_layers(&pixels);

    Report {
        platform: std::env::consts::OS,
        adapter: adapter_info.name,
        backend: format!("{:?}", adapter_info.backend),
        live2d_revision: LIVE2D_REVISION,
        workload: "synthetic 36-drawable live2d-wgpu workload composed through RuntimeDocument mixed Quad/HostTexture/Text UiScene with background HostTexture, in-card chrome, and a Live2D foreground band",
        viewport: [WIDTH, HEIGHT],
        live2d_target: [LIVE2D_SIZE, LIVE2D_SIZE],
        warmup_iterations: WARMUP,
        measured_iterations: ITERATIONS,
        ui_only: distribution(&ui_only),
        live2d_only: distribution(&live2d_only),
        ui_live2d_composed: distribution(&composed),
        screenshot: screenshot_path.display().to_string(),
        screenshot_checksum: pixels.iter().fold(0_u64, |sum, byte| {
            sum.wrapping_mul(16_777_619).wrapping_add(u64::from(*byte))
        }),
        screenshot_distinct_colors: distinct_colors,
    }
}

struct SubmittedScene {
    cpu_ms: f64,
    submission: wgpu::SubmissionIndex,
}

fn wait_gpu(device: &wgpu::Device, submission: wgpu::SubmissionIndex, message: &'static str) {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .expect(message);
}

fn submit_scene(
    painter: &mut SceneWgpuPainter,
    scene: &UiScene,
    textures: Option<&HostTextureRegistry>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::TextureView,
) -> SubmittedScene {
    let colors = ThemeMode::Dark.colors();
    let cpu_started = Instant::now();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui live2d acceptance paint"),
    });
    painter
        .paint(
            scene,
            &mut encoder,
            target,
            ScenePaintViewport {
                logical_size: [WIDTH as f32, HEIGHT as f32],
                physical_size: [WIDTH, HEIGHT],
                scale_factor: 1.0,
                scene_origin: [0.0, 0.0],
                target_origin: [0.0, 0.0],
                clear_color: [
                    colors.background.r,
                    colors.background.g,
                    colors.background.b,
                    colors.background.a,
                ],
                clear: true,
            },
            textures,
            None,
        )
        .expect("acceptance scene must paint");
    let cpu_ms = elapsed_ms(cpu_started);
    let submission = queue.submit([encoder.finish()]);
    SubmittedScene { cpu_ms, submission }
}

fn paint_scene(
    painter: &mut SceneWgpuPainter,
    scene: &UiScene,
    textures: Option<&HostTextureRegistry>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::TextureView,
) -> (Sample, wgpu::SubmissionIndex) {
    let total_started = Instant::now();
    let submitted = submit_scene(painter, scene, textures, device, queue, target);
    let wait_started = Instant::now();
    wait_gpu(
        device,
        submitted.submission.clone(),
        "UI GPU frame completes",
    );
    (
        Sample {
            cpu_ms: submitted.cpu_ms,
            submit_to_complete_ms: elapsed_ms(wait_started),
            total_ms: elapsed_ms(total_started),
        },
        submitted.submission,
    )
}

fn screenshot_target(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<u8> {
    let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui live2d acceptance readback"),
    });
    offscreen::readback(device, queue, encoder, texture, Size::new(WIDTH, HEIGHT))
        .expect("acceptance screenshot readback")
}

/// Opaque Selected fill; Primary AccentSoft would show the magenta slot through.
fn in_card_chrome_button() -> RuntimeButton {
    RuntimeButton::new("Start").kind(ButtonKind::Selected)
}

fn acceptance_scene(layers: Option<&LayerTextures>) -> (UiScene, HostTextureRegistry) {
    let document_id = DocumentId::new(1).expect("acceptance document");
    let mut document = RuntimeDocument::new(document_id);
    let header = document
        .context_mut()
        .create_component(document_id, RuntimeText::new("Live2D Composition"))
        .expect("header");
    let live = document
        .context_mut()
        .create_component(document_id, RuntimeText::new("LIVE"))
        .expect("live badge");
    let root = document
        .context_mut()
        .create_component(document_id, RuntimeCard::new())
        .expect("runtime preview root");
    let background = layers.map(|_| {
        document
            .context_mut()
            .create_component(document_id, GpuTextureView::new("live2d.bg"))
            .expect("background host texture")
    });
    let caption = document
        .context_mut()
        .create_component(
            document_id,
            RuntimeText::new(if layers.is_some() {
                "Program"
            } else {
                "Preview"
            }),
        )
        .expect("runtime preview caption");
    let chrome = document
        .context_mut()
        .create_component(document_id, in_card_chrome_button())
        .expect("in-card chrome");
    let foreground = layers.map(|_| {
        document
            .context_mut()
            .create_component(document_id, GpuTextureView::new("live2d.fg"))
            .expect("foreground host texture")
    });
    if let Some(background) = background {
        document
            .context_mut()
            .append_child(root, background)
            .expect("append background host texture");
    }
    document
        .context_mut()
        .append_child(root, caption)
        .expect("append caption");
    document
        .context_mut()
        .append_child(root, chrome)
        .expect("append in-card chrome");
    if let Some(foreground) = foreground {
        document
            .context_mut()
            .append_child(root, foreground)
            .expect("append foreground host texture");
    }
    let preview = document
        .context_mut()
        .create_component(document_id, RuntimeButton::new("Preview"))
        .expect("preview button");
    let take = document
        .context_mut()
        .create_component(
            document_id,
            RuntimeButton::new("Take").kind(ButtonKind::Primary),
        )
        .expect("take button");
    let fps = document
        .context_mut()
        .create_component(document_id, RuntimeText::new("60 fps · GPU texture"))
        .expect("fps");

    let mut mutations = MutationQueue::new();
    mutations.write_layout(
        header.stable_id(),
        LayoutBox {
            x: 20.0,
            y: 12.0,
            width: 320.0,
            height: 28.0,
        },
    );
    mutations.write_layout(
        live.stable_id(),
        LayoutBox {
            x: 820.0,
            y: 12.0,
            width: 60.0,
            height: 28.0,
        },
    );
    mutations.write_layout(
        root.stable_id(),
        LayoutBox {
            x: PREVIEW_X,
            y: PREVIEW_Y,
            width: PREVIEW_SIZE,
            height: PREVIEW_SIZE,
        },
    );
    if let Some(background) = background {
        mutations.write_layout(
            background.stable_id(),
            LayoutBox {
                x: PREVIEW_X,
                y: PREVIEW_Y,
                width: PREVIEW_SIZE,
                height: PREVIEW_SIZE,
            },
        );
        mutations.set_custom_render(
            background.stable_id(),
            Some(CustomRenderNode {
                renderer: "nana.host-texture".into(),
                resource: "live2d.bg".into(),
                revision: layers.map_or(0, |layers| layers.background.version()),
            }),
        );
    }
    mutations.write_layout(
        caption.stable_id(),
        LayoutBox {
            x: 212.0,
            y: 82.0,
            width: 280.0,
            height: 28.0,
        },
    );
    mutations.write_layout(
        chrome.stable_id(),
        LayoutBox {
            x: CHROME_BUTTON_X,
            y: CHROME_BUTTON_Y,
            width: CHROME_BUTTON_WIDTH,
            height: CHROME_BUTTON_HEIGHT,
        },
    );
    if let Some(foreground) = foreground {
        mutations.write_layout(
            foreground.stable_id(),
            LayoutBox {
                x: PREVIEW_X,
                y: PREVIEW_Y + PREVIEW_SIZE - FG_BAND_HEIGHT,
                width: PREVIEW_SIZE,
                height: FG_BAND_HEIGHT,
            },
        );
        mutations.set_custom_render(
            foreground.stable_id(),
            Some(CustomRenderNode {
                renderer: "nana.host-texture".into(),
                resource: "live2d.fg".into(),
                revision: layers.map_or(0, |layers| layers.foreground.version()),
            }),
        );
    }
    mutations.write_layout(
        preview.stable_id(),
        LayoutBox {
            x: 20.0,
            y: 590.0,
            width: 88.0,
            height: 32.0,
        },
    );
    mutations.write_layout(
        take.stable_id(),
        LayoutBox {
            x: 116.0,
            y: 590.0,
            width: 72.0,
            height: 32.0,
        },
    );
    mutations.write_layout(
        fps.stable_id(),
        LayoutBox {
            x: 700.0,
            y: 594.0,
            width: 180.0,
            height: 24.0,
        },
    );
    document
        .context_mut()
        .commit_mutations(mutations)
        .expect("commit runtime preview");
    document
        .flush_with(|context, work| {
            context.shape_text(&work.text, &mut NanaTextShaper::default())?;
            Ok(())
        })
        .expect("extract runtime preview scene");

    let registry = HostTextureRegistry::new();
    if let Some(layers) = layers {
        registry.register(
            "live2d.bg",
            layers.background.clone(),
            LIVE2D_SIZE,
            LIVE2D_SIZE,
            HostTextureAlphaMode::Premultiplied,
        );
        registry.register(
            "live2d.fg",
            layers.foreground.clone(),
            LIVE2D_SIZE,
            LIVE2D_SIZE,
            HostTextureAlphaMode::Premultiplied,
        );
    }
    (document.scene().clone(), registry)
}

fn live2d_resource_scene(revision: u64) -> Arc<UiScene> {
    #[derive(Debug)]
    struct ResourceNode;

    let document_id = DocumentId::new(2).expect("producer document");
    let mut document = RuntimeDocument::new(document_id);
    let node = document
        .context_mut()
        .create_view(
            document_id,
            NodeKind::Element {
                tag: "live2d-resource".into(),
            },
            ResourceNode,
        )
        .expect("producer resource node");
    let mut mutations = MutationQueue::new();
    mutations.write_layout(
        node.stable_id(),
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: LIVE2D_SIZE as f32,
            height: LIVE2D_SIZE as f32,
        },
    );
    mutations.set_custom_render(
        node.stable_id(),
        Some(CustomRenderNode {
            renderer: "nana.host-texture".into(),
            resource: "live2d".into(),
            revision,
        }),
    );
    document
        .context_mut()
        .commit_mutations(mutations)
        .expect("commit producer resource");
    document
        .flush_with(|_, _| Ok(()))
        .expect("extract producer resource scene");
    document.shared_scene()
}

fn synthetic_model() -> live2d_core::ModelSnapshot {
    let quad = |x: f32, y: f32, size: f32| {
        vec![
            Vertex {
                position: [x - size, y - size],
                uv: [0.0, 1.0],
            },
            Vertex {
                position: [x + size, y - size],
                uv: [1.0, 1.0],
            },
            Vertex {
                position: [x + size, y + size],
                uv: [1.0, 0.0],
            },
            Vertex {
                position: [x - size, y + size],
                uv: [0.0, 0.0],
            },
        ]
    };
    let indices = vec![0, 1, 2, 0, 2, 3];
    let mut drawables = Vec::new();
    for mask in 0..4 {
        drawables.push(
            DrawableBuilder::new(format!("mask-{mask}"), mask)
                .vertices(quad(-0.55 + mask as f32 * 0.36, 0.0, 0.28))
                .indices(indices.clone())
                .opacity(0.01)
                .build(),
        );
    }
    for index in 0_usize..32 {
        let column = (index % 8) as f32;
        let row = (index / 8) as f32;
        drawables.push(
            DrawableBuilder::new(format!("mesh-{index}"), index as i32 + 4)
                .vertices(quad(-0.7 + column * 0.2, -0.6 + row * 0.4, 0.15))
                .indices(indices.clone())
                .clipping(ClippingInfo {
                    drawable_ids: vec![DrawableId::from(format!("mask-{}", index % 4))],
                    inverted: index.is_multiple_of(7),
                })
                .blend_color(BlendColor {
                    multiply: [0.35 + column * 0.07, 0.45 + row * 0.12, 0.9, 1.0],
                    screen: [0.08, 0.02 * column, 0.04 * row, 1.0],
                })
                .build(),
        );
    }
    let render_objects = (0..32)
        .map(|index| RenderObject::Drawable(DrawableId::from(format!("mesh-{index}"))))
        .collect::<Vec<_>>();
    let mut snapshot = SnapshotBuilder::new()
        .model_key("nanaui-live2d-acceptance")
        .drawables(drawables)
        .render_objects(render_objects)
        .build();
    snapshot.textures = vec![TextureAsset {
        width: 2,
        height: 2,
        rgba: vec![255; 16],
    }];
    snapshot
}

fn distribution(samples: &[Sample]) -> Distribution {
    Distribution {
        p50: percentile(samples, 0.50),
        p95: percentile(samples, 0.95),
        p99: percentile(samples, 0.99),
        max: max_sample(samples),
        frame_budget_ms: 16.67,
        frame_budget_misses: samples
            .iter()
            .filter(|sample| sample.total_ms > 16.67)
            .count(),
    }
}

fn max_sample(samples: &[Sample]) -> Sample {
    let select = |field: fn(&Sample) -> f64| {
        samples
            .iter()
            .map(field)
            .max_by(f64::total_cmp)
            .unwrap_or(0.0)
    };
    Sample {
        cpu_ms: select(|sample| sample.cpu_ms),
        submit_to_complete_ms: select(|sample| sample.submit_to_complete_ms),
        total_ms: select(|sample| sample.total_ms),
    }
}

fn percentile(samples: &[Sample], quantile: f64) -> Sample {
    let select = |field: fn(&Sample) -> f64| {
        let mut values = samples.iter().map(field).collect::<Vec<_>>();
        values.sort_by(f64::total_cmp);
        values[((values.len() - 1) as f64 * quantile).ceil() as usize]
    };
    Sample {
        cpu_ms: select(|sample| sample.cpu_ms),
        submit_to_complete_ms: select(|sample| sample.submit_to_complete_ms),
        total_ms: select(|sample| sample.total_ms),
    }
}

fn distinct_colors(pixels: &[u8]) -> usize {
    let mut colors = pixels
        .chunks_exact(4)
        .map(|pixel| u32::from_le_bytes(pixel.try_into().expect("RGBA pixel")))
        .collect::<Vec<_>>();
    colors.sort_unstable();
    colors.dedup();
    colors.len()
}

fn solid_host_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    color: wgpu::Color,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Live2D acceptance background fill"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Live2D acceptance background fill"),
    });
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Live2D acceptance background fill"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    queue.submit([encoder.finish()]);
    (texture, view)
}

fn assert_chrome_between_host_texture_layers(pixels: &[u8]) {
    let background = pixel_at(
        pixels,
        (PREVIEW_X + 26.0) as u32,
        (PREVIEW_Y + 336.0) as u32,
    );
    let chrome = pixel_at(pixels, CHROME_FILL_SAMPLE_X, CHROME_FILL_SAMPLE_Y);
    let foreground = pixel_at(
        pixels,
        (PREVIEW_X + PREVIEW_SIZE / 2.0) as u32,
        (PREVIEW_Y + PREVIEW_SIZE - FG_BAND_HEIGHT / 2.0) as u32,
    );
    assert!(
        is_magenta_fill(background),
        "background HostTexture slot must keep BG_FILL magenta, got {background:?}"
    );
    assert!(
        is_selected_chrome_fill(chrome),
        "in-card Selected Button fill must be the opaque Selected theme, got {chrome:?}"
    );
    assert!(
        !is_magenta_fill(foreground) && !is_selected_chrome_fill(foreground),
        "foreground Live2D HostTexture band must be neither magenta nor chrome, got {foreground:?}"
    );
}

fn is_magenta_fill(pixel: [u8; 4]) -> bool {
    channel_near(pixel[0], BG_FILL_RGBA[0])
        && channel_near(pixel[1], BG_FILL_RGBA[1])
        && channel_near(pixel[2], BG_FILL_RGBA[2])
        && pixel[3] >= 240
}

fn is_selected_chrome_fill(pixel: [u8; 4]) -> bool {
    let fill = ThemeMode::Dark.colors().selected;
    let expected = [
        (fill.r * 255.0).round() as u8,
        (fill.g * 255.0).round() as u8,
        (fill.b * 255.0).round() as u8,
    ];
    fill.a > 0.99
        && channel_near(pixel[0], expected[0])
        && channel_near(pixel[1], expected[1])
        && channel_near(pixel[2], expected[2])
        && pixel[3] >= 240
}

fn channel_near(actual: u8, expected: u8) -> bool {
    (i16::from(actual) - i16::from(expected)).abs() <= CHROME_FILL_CHANNEL_SLACK
}

fn pixel_at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let index = ((y * WIDTH + x) * 4) as usize;
    pixels[index..index + 4]
        .try_into()
        .expect("RGBA pixel inside screenshot")
}

fn elapsed_ms(started: Instant) -> f64 {
    duration_ms(started.elapsed())
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
