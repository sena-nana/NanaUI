use std::time::Instant;

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Pixels, Point, Size, Theme};
use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::core::{Event, mouse, renderer, shell, window};
use iced_winit::futures::futures::executor;
use iced_winit::runtime::UserInterface;
use iced_winit::runtime::user_interface;
use nana_ui::widgets::{list_item_style, panel_style, scrollable_style, vertical_scrollbar};
use nana_ui::{ThemeMode, UI_METRICS, status_indicator, ui_font, ui_font_sources};

use crate::report::{AdapterReport, BenchmarkReport, CaseReport, Sample};

const VIEWPORT_WIDTH: u32 = 900;
const VIEWPORT_HEIGHT: u32 = 640;
const WARMUP_ITERATIONS: usize = 10;
const ITERATIONS: usize = 60;

pub fn run() -> BenchmarkReport {
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
    .expect("benchmark must find a headless WGPU adapter");
    let adapter_info = adapter.get_info();
    let (device, queue) = executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nana-ui benchmark device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .expect("benchmark must create a WGPU device");
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let mut font_system = iced_wgpu::graphics::text::font_system()
        .write()
        .expect("font system");
    for source in ui_font_sources() {
        font_system.load_font(std::borrow::Cow::Borrowed(source));
    }
    drop(font_system);
    let engine = Engine::new(
        &adapter,
        device.clone(),
        queue,
        format,
        None,
        Shell::headless(),
    );
    let mut renderer = Renderer::new(
        engine,
        renderer::Settings {
            default_font: ui_font(iced::font::Weight::Normal),
            default_text_size: Pixels::from(13),
            metrics_hinting: true,
        },
    );
    let viewport = Viewport::with_physical_size(
        Size::new(VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
        renderer::Scale::default(),
    );
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui benchmark target"),
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
    let target = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let cases = [100, 500, 1_000]
        .into_iter()
        .map(|item_count| {
            benchmark_case(
                item_count,
                &device,
                &mut renderer,
                &target,
                &viewport,
                format,
            )
        })
        .collect();

    BenchmarkReport {
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        adapter: AdapterReport {
            name: adapter_info.name,
            backend: format!("{:?}", adapter_info.backend),
            device_type: format!("{:?}", adapter_info.device_type),
        },
        iterations: ITERATIONS,
        warmup_iterations: WARMUP_ITERATIONS,
        viewport: [VIEWPORT_WIDTH, VIEWPORT_HEIGHT],
        cases,
    }
}

fn benchmark_case(
    item_count: usize,
    device: &wgpu::Device,
    renderer: &mut Renderer,
    target: &wgpu::TextureView,
    viewport: &Viewport,
    format: wgpu::TextureFormat,
) -> CaseReport {
    let mut cache = user_interface::Cache::new();
    let mut samples = Vec::with_capacity(ITERATIONS);
    let mut render = RenderContext {
        device,
        renderer,
        target,
        viewport,
        format,
    };

    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        let (next_cache, sample) = render_iteration(item_count, iteration, cache, &mut render);
        cache = next_cache;
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }

    CaseReport::from_samples(item_count, &samples)
}

struct RenderContext<'a> {
    device: &'a wgpu::Device,
    renderer: &'a mut Renderer,
    target: &'a wgpu::TextureView,
    viewport: &'a Viewport,
    format: wgpu::TextureFormat,
}

fn render_iteration(
    item_count: usize,
    iteration: usize,
    cache: user_interface::Cache,
    render: &mut RenderContext<'_>,
) -> (user_interface::Cache, Sample) {
    let started = Instant::now();
    let view = list_view(item_count, iteration % item_count);
    let view_construction_ms = elapsed_ms(started);

    let started = Instant::now();
    let mut interface =
        UserInterface::build(view, render.viewport.logical_size(), cache, render.renderer);
    let layout_diff_ms = elapsed_ms(started);

    let direction = if iteration % 20 < 10 { -2.0 } else { 2.0 };
    let events = [Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Lines {
            x: 0.0,
            y: direction,
        },
    })];
    let cursor = mouse::Cursor::Available(Point::new(450.0, 320.0));
    let mut messages = Vec::new();
    let window = window::Headless;
    let waker = shell::Waker::noop();
    let started = Instant::now();
    let _ = interface.update(
        &window,
        &waker,
        &events,
        cursor,
        render.renderer,
        &mut messages,
    );
    let event_update_ms = elapsed_ms(started);

    let colors = ThemeMode::Dark.colors();
    let started = Instant::now();
    interface.draw(
        render.renderer,
        &Theme::Dark,
        &renderer::Style {
            text_color: colors.text,
        },
        cursor,
    );
    let draw_cpu_ms = elapsed_ms(started);
    let cache = interface.into_cache();

    let started = Instant::now();
    let submission = render.renderer.present(
        Some(colors.background),
        render.format,
        render.target,
        render.viewport,
    );
    render
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .expect("benchmark GPU work must complete");
    let gpu_submit_wait_ms = elapsed_ms(started);

    (
        cache,
        Sample {
            view_construction_ms,
            layout_diff_ms,
            event_update_ms,
            draw_cpu_ms,
            gpu_submit_wait_ms,
        },
    )
}

fn list_view(item_count: usize, selected: usize) -> Element<'static, usize, Theme, Renderer> {
    let colors = ThemeMode::Dark.colors();
    let mut items = column![].spacing(4);
    for index in 0..item_count {
        let is_selected = index == selected;
        items = items.push(
            button(
                row![
                    status_indicator(
                        is_selected,
                        10.0,
                        if is_selected {
                            colors.accent
                        } else {
                            colors.faint
                        },
                    ),
                    text(format!("节点 {}", index + 1)).size(13),
                    space().width(Length::Fill),
                    text(format!("{:04}", index + 1))
                        .size(11)
                        .color(colors.muted),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fixed(UI_METRICS.selection_height))
            .padding([
                UI_METRICS.list_item_padding_y,
                UI_METRICS.list_item_padding_x,
            ])
            .on_press(index)
            .style(list_item_style(colors, is_selected)),
        );
    }

    container(
        column![
            text(format!("{item_count} 个节点"))
                .size(12)
                .color(colors.muted),
            scrollable(items)
                .direction(vertical_scrollbar())
                .style(scrollable_style(colors))
                .height(Length::Fill),
        ]
        .spacing(8),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(14)
    .style(panel_style(colors))
    .into()
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}
