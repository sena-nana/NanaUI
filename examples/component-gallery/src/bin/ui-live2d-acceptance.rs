//! macOS acceptance for the supported Live2D -> host texture -> NanaUI path.
//!
//! This binary intentionally lives outside NanaUI's public API. Live2D owns
//! model evaluation and rendering; NanaUI only samples a host-owned texture.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::widget::{column, container, row, space, stack, text};
use iced::{Alignment, Color, Element, Length, Pixels, Size, Theme};
use iced_wgpu::graphics::{Antialiasing, Shell, Viewport};
use iced_wgpu::{Engine, Renderer as IcedRenderer, wgpu};
use iced_winit::core::{Event, mouse, renderer, shell, window};
use iced_winit::futures::futures::executor;
use iced_winit::runtime::{UserInterface, user_interface};
use live2d_core::{
    BlendColor, ClippingInfo, DrawableId, FrameDirtyFlags, ModelDynamicFrame, ModelGeometryFrame,
    ModelStaticData, RenderObject, RuntimeFrame, TextureAsset, Vertex,
};
use live2d_test_support::{DrawableBuilder, SnapshotBuilder};
use live2d_wgpu::{
    RegistrationRequest, RenderTarget, RenderView, Renderer as Live2dRenderer, RendererOptions,
    SubmissionBatch, SubmissionToken,
};
use nana_ui::compatibility::Button;
use nana_ui::runtime::{
    Card as RuntimeCard, CustomRenderNode, DocumentId, LayoutBox, MutationQueue, NodeKind,
    RuntimeDocument, Text as RuntimeText, UiScene,
};
use nana_ui::widgets::{ButtonKind, panel_style};
use nana_ui::{
    HostTexture, HostTextureAlphaMode, HostTextureRegistry, IcedSceneView,
    SceneResourceEncodeContext, SceneResourceProducer, SceneResourceProducerRegistry, ThemeMode,
    ThemeModeExt, UI_BASE_TEXT_SIZE, ui_font, ui_font_sources,
};
use serde::Serialize;

#[path = "ui_snapshots/write.rs"]
mod write;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;
const LIVE2D_SIZE: u32 = 512;
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

struct PendingLive2dSubmission {
    token: SubmissionToken,
    total_started: Instant,
    submit_started: Instant,
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
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .expect("Scene-managed Live2D GPU frame completes");
        let mut state = self.state.lock().expect("Live2D producer state");
        let pending = state.pending.take().expect("submitted Live2D frame");
        let submit_to_complete = pending.submit_started.elapsed();
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
        let total_started = Instant::now();
        let cpu_started = Instant::now();
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
            total_started,
            submit_started: Instant::now(),
            cpu_ms: elapsed_ms(cpu_started),
        });
        Ok(())
    }

    fn submitted(
        &self,
        _node: &CustomRenderNode,
        _device: &wgpu::Device,
        _submission: wgpu::SubmissionIndex,
    ) {
        let mut state = self.state.lock().expect("Live2D producer state");
        let pending = state
            .pending
            .as_mut()
            .expect("Live2D producer encoded a submission");
        pending.submit_started = Instant::now();
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
    let adapter = executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("acceptance requires a WGPU adapter");
    let adapter_info = adapter.get_info();
    let (device, queue) = executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("NanaUI Live2D acceptance device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .expect("acceptance requires a WGPU device");

    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    load_fonts();
    let engine = Engine::new(
        &adapter,
        device.clone(),
        queue.clone(),
        format,
        Some(Antialiasing::MSAAx4),
        Shell::headless(),
    );
    let mut ui_renderer = IcedRenderer::new(
        engine,
        renderer::Settings {
            default_font: ui_font(iced::font::Weight::Normal),
            default_text_size: Pixels::from(UI_BASE_TEXT_SIZE),
            metrics_hinting: true,
        },
    );
    let viewport =
        Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), renderer::Scale::default());
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
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

    let mut ui_only_cache = user_interface::Cache::new();
    let mut composed_cache = user_interface::Cache::new();
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
                    let (cache, sample) = render_ui(
                        None,
                        &device,
                        &mut ui_renderer,
                        &viewport,
                        &ui_target_view,
                        format,
                        std::mem::take(&mut ui_only_cache),
                    );
                    ui_only_cache = cache;
                    ui_sample = Some(sample);
                }
                Workload::Live2d => {
                    let submission = resource_producers
                        .encode_scene(&resource_scene, &device, &queue)
                        .expect("encode graph-managed Live2D resource")
                        .expect("Live2D resource producer is registered");
                    live2d_sample = Some(producer.complete(&device, submission));
                }
                Workload::Composed => {
                    resource_producers
                        .encode_scene(&resource_scene, &device, &queue)
                        .expect("encode graph-managed composed Live2D resource")
                        .expect("Live2D resource producer is registered");
                    let ui_cpu_started = Instant::now();
                    let (cache, submission) = draw_ui(
                        Some(host_texture.clone()),
                        &mut ui_renderer,
                        &viewport,
                        &ui_target_view,
                        format,
                        std::mem::take(&mut composed_cache),
                    );
                    let ui_cpu_ms = elapsed_ms(ui_cpu_started);
                    let mut sample = producer.complete(&device, submission);
                    sample.cpu_ms += ui_cpu_ms;
                    composed_cache = cache;
                    composed_sample = Some(sample);
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
    let (_, final_submission) = draw_ui(
        Some(host_texture.clone()),
        &mut ui_renderer,
        &viewport,
        &ui_target_view,
        format,
        composed_cache,
    );
    let _ = producer.complete(&device, final_submission);
    let pixels = screenshot_ui(host_texture.clone(), &mut ui_renderer, &viewport);
    write::png(screenshot_path, Size::new(WIDTH, HEIGHT), &pixels).expect("write screenshot");
    let distinct_colors = distinct_colors(&pixels);
    assert!(
        distinct_colors > 32,
        "composed screenshot must contain rendered detail"
    );

    Report {
        platform: std::env::consts::OS,
        adapter: adapter_info.name,
        backend: format!("{:?}", adapter_info.backend),
        live2d_revision: LIVE2D_REVISION,
        workload: "synthetic 36-drawable live2d-wgpu workload composed through RuntimeDocument mixed Quad/HostTexture/Text UiScene",
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

fn render_ui(
    texture: Option<HostTexture>,
    device: &wgpu::Device,
    renderer: &mut IcedRenderer,
    viewport: &Viewport,
    target: &wgpu::TextureView,
    format: wgpu::TextureFormat,
    cache: user_interface::Cache,
) -> (user_interface::Cache, Sample) {
    let total_started = Instant::now();
    let cpu_started = Instant::now();
    let (cache, submission) = draw_ui(texture, renderer, viewport, target, format, cache);
    let cpu_ms = elapsed_ms(cpu_started);
    let wait_started = Instant::now();
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .expect("UI GPU frame completes");
    (
        cache,
        Sample {
            cpu_ms,
            submit_to_complete_ms: elapsed_ms(wait_started),
            total_ms: elapsed_ms(total_started),
        },
    )
}

fn draw_ui(
    texture: Option<HostTexture>,
    renderer: &mut IcedRenderer,
    viewport: &Viewport,
    target: &wgpu::TextureView,
    format: wgpu::TextureFormat,
    cache: user_interface::Cache,
) -> (user_interface::Cache, wgpu::SubmissionIndex) {
    let mut interface = UserInterface::build(ui(texture), viewport.logical_size(), cache, renderer);
    let window = window::Headless;
    let waker = shell::Waker::noop();
    let _ = interface.update(
        &window,
        &waker,
        &[Event::Window(
            window::Event::RedrawRequested(Instant::now()),
        )],
        mouse::Cursor::Unavailable,
        renderer,
        &mut shell::Bus::new(),
    );
    let colors = ThemeMode::Dark.colors();
    interface.draw(
        renderer,
        &Theme::Dark,
        &renderer::Style {
            text_color: colors.text,
        },
        mouse::Cursor::Unavailable,
    );
    let cache = interface.into_cache();
    let submission = renderer.present(Some(colors.background), format, target, viewport);
    (cache, submission)
}

fn screenshot_ui(
    texture: HostTexture,
    renderer: &mut IcedRenderer,
    viewport: &Viewport,
) -> Vec<u8> {
    let mut interface = UserInterface::build(
        ui(Some(texture)),
        viewport.logical_size(),
        user_interface::Cache::new(),
        renderer,
    );
    let window = window::Headless;
    let waker = shell::Waker::noop();
    let _ = interface.update(
        &window,
        &waker,
        &[Event::Window(
            window::Event::RedrawRequested(Instant::now()),
        )],
        mouse::Cursor::Unavailable,
        renderer,
        &mut shell::Bus::new(),
    );
    let colors = ThemeMode::Dark.colors();
    interface.draw(
        renderer,
        &Theme::Dark,
        &renderer::Style {
            text_color: colors.text,
        },
        mouse::Cursor::Unavailable,
    );
    let pixels = renderer.screenshot(viewport, colors.background);
    drop(interface);
    pixels
}

fn ui(texture: Option<HostTexture>) -> Element<'static, (), Theme, IcedRenderer> {
    let colors = ThemeMode::Dark.colors();
    let preview = runtime_preview(texture);
    let header = container(
        row![
            text("Live2D Composition").size(18),
            space().width(Length::Fill),
            text("LIVE").color(Color::from_rgb8(255, 92, 112)),
        ]
        .align_y(Alignment::Center),
    )
    .height(48)
    .padding([0, 20])
    .style(move |_theme| {
        iced::widget::container::Style::default()
            .background(Color::from_rgba8(19, 22, 28, 0.94))
            .color(colors.text)
    });
    let footer = container(
        row![
            Button::label("Preview").on_press(()).view(colors),
            Button::label("Take")
                .kind(ButtonKind::Primary)
                .on_press(())
                .view(colors),
            space().width(Length::Fill),
            text("60 fps · GPU texture")
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .height(64)
    .padding([10, 20])
    .style(move |_theme| {
        iced::widget::container::Style::default()
            .background(Color::from_rgba8(19, 22, 28, 0.94))
            .color(colors.text)
    });
    let chrome = column![header, space().height(Length::Fill), footer]
        .width(Length::Fill)
        .height(Length::Fill);
    let preview = container(preview)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(56)
        .style(panel_style(colors));
    stack![preview, chrome]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn runtime_preview(texture: Option<HostTexture>) -> Element<'static, (), Theme, IcedRenderer> {
    #[derive(Debug)]
    struct TextureNode;

    let document_id = DocumentId::new(1).expect("acceptance document");
    let mut document = RuntimeDocument::new(document_id);
    let root = document
        .context_mut()
        .create_component(document_id, RuntimeCard::new())
        .expect("runtime preview root");
    let texture_node = texture.as_ref().map(|_| {
        document
            .context_mut()
            .create_view(
                document_id,
                NodeKind::Element {
                    tag: "host-texture".into(),
                },
                TextureNode,
            )
            .expect("runtime host texture node")
    });
    let caption = document
        .context_mut()
        .create_component(
            document_id,
            RuntimeText::new(if texture.is_some() {
                "Program"
            } else {
                "Preview"
            }),
        )
        .expect("runtime preview caption");
    if let Some(texture_node) = texture_node {
        document
            .context_mut()
            .append_child(root, texture_node)
            .expect("append host texture");
    }
    document
        .context_mut()
        .append_child(root, caption)
        .expect("append caption");

    let mut mutations = MutationQueue::new();
    mutations.write_layout(
        root.stable_id(),
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 512.0,
            height: 512.0,
        },
    );
    if let Some(texture_node) = texture_node {
        mutations.write_layout(
            texture_node.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 512.0,
                height: 512.0,
            },
        );
        mutations.set_custom_render(
            texture_node.stable_id(),
            Some(CustomRenderNode {
                renderer: "nana.host-texture".into(),
                resource: "live2d".into(),
                revision: texture.as_ref().map_or(0, HostTexture::version),
            }),
        );
    }
    mutations.write_layout(
        caption.stable_id(),
        LayoutBox {
            x: 18.0,
            y: 18.0,
            width: 280.0,
            height: 28.0,
        },
    );
    document
        .context_mut()
        .commit_mutations(mutations)
        .expect("commit runtime preview");
    document
        .flush_with(|_, _| Ok(()))
        .expect("extract runtime preview scene");

    let registry = texture.map(|texture| {
        let registry = HostTextureRegistry::new();
        registry.register(
            "live2d",
            texture,
            LIVE2D_SIZE,
            LIVE2D_SIZE,
            HostTextureAlphaMode::Premultiplied,
        );
        registry
    });
    IcedSceneView::from_shared(document.shared_scene(), registry, Size::new(512.0, 512.0))
        .expect("runtime preview scene must be paintable")
        .into()
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

fn load_fonts() {
    let mut font_system = iced_wgpu::graphics::text::font_system()
        .write()
        .expect("font system");
    for source in ui_font_sources() {
        font_system.load_font(std::borrow::Cow::Borrowed(source));
    }
}

fn elapsed_ms(started: Instant) -> f64 {
    duration_ms(started.elapsed())
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
