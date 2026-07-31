use std::time::Instant;

use iced::widget::{column, container, scrollable, text};
use iced::{Element, Length, Pixels, Point, Size, Theme};
use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::core::{Event, mouse, renderer, shell, window};
use iced_winit::futures::futures::executor;
use iced_winit::runtime::UserInterface;
use iced_winit::runtime::user_interface;
use nana_ui::widgets::{panel_style, scrollable_style, vertical_scrollbar};
use nana_ui::{
    AnchoredMenuPosition, ContextMenuEvent, ContextMenuHost, ContextMenuItem, Dropdown,
    DropdownEvent, DropdownOption, GalleryMessage, GallerySection, GalleryState, ListItem,
    RegionId, RegionRole, RegionState, SearchDropdown, SearchDropdownOption, SearchDropdownState,
    SettingsTabId, ThemeMode, UI_BASE_TEXT_SIZE, WorkspaceAction, WorkspaceController,
    WorkspaceLayout, WorkspaceRegions, status_indicator, ui_font, ui_font_sources, workspace_view,
};

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
            default_text_size: Pixels::from(UI_BASE_TEXT_SIZE),
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
    let mut render = RenderContext {
        device: &device,
        renderer: &mut renderer,
        target: &target,
        viewport: &viewport,
        format,
    };

    let mut cases: Vec<_> = [100, 500, 1_000]
        .into_iter()
        .map(|item_count| benchmark_case(item_count, &mut render))
        .collect();
    for (name, setup) in [
        (
            "gallery-controls",
            gallery_controls as fn(&mut GalleryState, usize),
        ),
        ("gallery-surfaces", gallery_surfaces),
        ("gallery-feedback", gallery_feedback),
        ("gallery-workspace", gallery_workspace),
        ("gallery-settings-appearance", gallery_settings_appearance),
        ("gallery-settings-workspace", gallery_settings_workspace),
        ("gallery-settings-about", gallery_settings_about),
        ("gallery-dialog", gallery_dialog),
        ("gallery-popover", gallery_popover),
        ("gallery-context-menu", gallery_context_menu),
        ("gallery-image-viewer", gallery_image_viewer),
    ] {
        cases.push(benchmark_gallery_case(name, setup, &mut render));
    }
    cases.push(benchmark_stateful_case(
        "dropdown-500",
        500,
        (0..500)
            .map(|index| DropdownOption::new(index, format!("选项 {}", index + 1)))
            .collect::<Vec<_>>(),
        |_, _| {},
        |options, iteration| {
            container(
                Dropdown::single(
                    Some(iteration % options.len()),
                    options.iter().cloned(),
                    |event| match event {
                        DropdownEvent::Select(value) | DropdownEvent::Toggle(value) => value,
                        DropdownEvent::Opened | DropdownEvent::Closed => 0,
                    },
                )
                .width(Length::Fixed(320.0))
                .view(ThemeMode::Dark.colors()),
            )
            .center(Length::Fill)
            .into()
        },
        &mut render,
    ));
    cases.push(benchmark_stateful_case(
        "search-dropdown-200",
        200,
        SearchDropdownState::new((0..200).map(|index| {
            SearchDropdownOption::new(index, format!("搜索结果 {}", index + 1))
                .hint(format!("分组 {}", index % 8))
        })),
        |_, _| {},
        |state, _iteration| {
            let control = container(
                SearchDropdown::new(state, None, |value| value)
                    .placeholder("搜索")
                    .on_input(|_| 0)
                    .view(ThemeMode::Dark.colors()),
            )
            .width(Length::Fixed(320.0));
            container(control).center(Length::Fill).into()
        },
        &mut render,
    ));
    cases.push(benchmark_stateful_case(
        "context-menu-120",
        120,
        (0..120)
            .map(|index| {
                ContextMenuItem::new(index, format!("操作 {}", index + 1))
                    .hint(format!("⌘{}", index % 10))
                    .keywords([format!("action-{index}")])
            })
            .collect::<Vec<_>>(),
        |_, _| {},
        |items, _| {
            ContextMenuHost::new(
                items,
                AnchoredMenuPosition::new(Point::new(340.0, 160.0)),
                Size::new(VIEWPORT_WIDTH as f32, VIEWPORT_HEIGHT as f32),
                |event| match event {
                    ContextMenuEvent::Select(value) => value,
                    ContextMenuEvent::Search(_)
                    | ContextMenuEvent::OpenSubmenu(_)
                    | ContextMenuEvent::Dismiss
                    | ContextMenuEvent::Interaction => 0,
                },
                ThemeMode::Dark.colors(),
            )
            .search("", true)
            .view()
        },
        &mut render,
    ));
    for region_count in [20, 50] {
        cases.push(benchmark_stateful_case(
            &format!("workspace-{region_count}-regions"),
            region_count,
            WorkspaceBenchmarkState::new(region_count),
            WorkspaceBenchmarkState::update,
            workspace_benchmark_view,
            &mut render,
        ));
    }

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

fn benchmark_case(item_count: usize, render: &mut RenderContext<'_>) -> CaseReport {
    let mut cache = user_interface::Cache::new();
    let mut samples = Vec::with_capacity(ITERATIONS);

    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        let (next_cache, sample) = render_iteration(item_count, iteration, cache, render);
        cache = next_cache;
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }

    CaseReport::from_samples(format!("list-{item_count}"), item_count, &samples)
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
    render_view(view, view_construction_ms, cache, render, iteration)
}

fn render_view<'a, Message>(
    view: Element<'a, Message, Theme, Renderer>,
    view_construction_ms: f64,
    cache: user_interface::Cache,
    render: &mut RenderContext<'_>,
    iteration: usize,
) -> (user_interface::Cache, Sample)
where
    Message: Clone + 'a,
{
    let started = Instant::now();
    let mut interface =
        UserInterface::build(view, render.viewport.logical_size(), cache, render.renderer);
    let layout_diff_ms = elapsed_ms(started);

    let direction = if iteration % 20 < 10 { -2.0 } else { 2.0 };
    let pointer = Point::new(450.0, 320.0);
    let events = [
        Event::Mouse(mouse::Event::CursorMoved { position: pointer }),
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines {
                x: 0.0,
                y: direction,
            },
        }),
    ];
    let cursor = mouse::Cursor::Available(pointer);
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

fn benchmark_gallery_case(
    name: &str,
    setup: fn(&mut GalleryState, usize),
    render: &mut RenderContext<'_>,
) -> CaseReport {
    let mut state = GalleryState::new();
    let mut cache = user_interface::Cache::new();
    let mut samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        setup(&mut state, iteration);
        let started = Instant::now();
        let view = state.view();
        let view_construction_ms = elapsed_ms(started);
        let (next_cache, sample) =
            render_view(view, view_construction_ms, cache, render, iteration);
        cache = next_cache;
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }
    CaseReport::from_samples(name, 0, &samples)
}

fn benchmark_stateful_case<State, Message, Update, View>(
    name: &str,
    item_count: usize,
    mut state: State,
    mut update: Update,
    view: View,
    render: &mut RenderContext<'_>,
) -> CaseReport
where
    Message: Clone + 'static,
    Update: FnMut(&mut State, usize),
    View: for<'a> Fn(&'a State, usize) -> Element<'a, Message, Theme, Renderer>,
{
    let mut cache = user_interface::Cache::new();
    let mut samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        update(&mut state, iteration);
        let started = Instant::now();
        let view = view(&state, iteration);
        let view_construction_ms = elapsed_ms(started);
        let (next_cache, sample) =
            render_view(view, view_construction_ms, cache, render, iteration);
        cache = next_cache;
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }
    CaseReport::from_samples(name, item_count, &samples)
}

fn gallery_controls(state: &mut GalleryState, iteration: usize) {
    state.update(GalleryMessage::SelectSection(GallerySection::Controls));
    state.update(GalleryMessage::SetSlider((iteration % 101) as u8));
}

fn gallery_surfaces(state: &mut GalleryState, iteration: usize) {
    state.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    state.update(GalleryMessage::SelectSurfaceCard(iteration % 4));
}

fn gallery_feedback(state: &mut GalleryState, _iteration: usize) {
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
}

fn gallery_workspace(state: &mut GalleryState, _iteration: usize) {
    state.update(GalleryMessage::SelectSection(GallerySection::Workspace));
}

fn open_settings(state: &mut GalleryState, tab: &str) {
    state.update(GalleryMessage::OpenSettings);
    state.update(GalleryMessage::SelectSettingsTab(SettingsTabId::from(tab)));
}

fn gallery_settings_appearance(state: &mut GalleryState, _iteration: usize) {
    open_settings(state, "appearance");
}

fn gallery_settings_workspace(state: &mut GalleryState, _iteration: usize) {
    open_settings(state, "workspace");
}

fn gallery_settings_about(state: &mut GalleryState, _iteration: usize) {
    open_settings(state, "about");
}

fn gallery_dialog(state: &mut GalleryState, iteration: usize) {
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    if iteration.is_multiple_of(2) {
        state.update(GalleryMessage::ToggleDialog);
    } else {
        state.update(GalleryMessage::DismissOverlay);
    }
}

fn gallery_popover(state: &mut GalleryState, iteration: usize) {
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    if iteration.is_multiple_of(2) {
        state.update(GalleryMessage::TogglePopover);
    } else {
        state.update(GalleryMessage::ClosePopover);
    }
}

fn gallery_context_menu(state: &mut GalleryState, iteration: usize) {
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    if iteration.is_multiple_of(2) {
        state.update(GalleryMessage::ToggleContextMenu);
    } else {
        state.update(GalleryMessage::DismissOverlay);
    }
}

fn gallery_image_viewer(state: &mut GalleryState, iteration: usize) {
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    if iteration.is_multiple_of(2) {
        state.update(GalleryMessage::ToggleImageViewer);
    } else {
        state.update(GalleryMessage::DismissOverlay);
    }
}

struct WorkspaceBenchmarkState {
    controller: WorkspaceController,
    region_ids: Vec<RegionId>,
}

impl WorkspaceBenchmarkState {
    fn new(region_count: usize) -> Self {
        let region_ids = (0..region_count)
            .map(|index| RegionId::custom(format!("benchmark-region-{index}")))
            .collect::<Vec<_>>();
        let states = region_ids.iter().enumerate().map(|(index, id)| {
            if index == 0 {
                RegionState::new(id.clone(), RegionRole::Resources)
                    .size(180.0)
                    .min_size(96.0)
                    .max_size(320.0)
                    .collapsible(true)
                    .resizable(true)
            } else {
                RegionState::new(id.clone(), RegionRole::Primary).fill_priority(1)
            }
        });
        let mut controller = WorkspaceController::with_layout(
            WorkspaceLayout::new(states).expect("benchmark regions are unique"),
        );
        controller.update(WorkspaceAction::WindowResized {
            width: VIEWPORT_WIDTH as f32,
            height: VIEWPORT_HEIGHT as f32,
        });
        Self {
            controller,
            region_ids,
        }
    }

    fn update(&mut self, iteration: usize) {
        let resizable = self.region_ids[0].clone();
        self.controller
            .update(WorkspaceAction::ResizeStart(resizable));
        self.controller
            .update(WorkspaceAction::ResizeMove { x: 0.0, y: 0.0 });
        self.controller.update(WorkspaceAction::ResizeMove {
            x: 140.0 + (iteration % 120) as f32,
            y: 0.0,
        });
        self.controller.update(WorkspaceAction::ResizeEnd);
        if iteration.is_multiple_of(5) {
            self.controller
                .update(WorkspaceAction::ToggleRegion(self.region_ids[1].clone()));
        }
    }
}

fn workspace_benchmark_view(
    state: &WorkspaceBenchmarkState,
    _iteration: usize,
) -> Element<'_, WorkspaceAction, Theme, Renderer> {
    let mut regions = WorkspaceRegions::new();
    for (index, id) in state.region_ids.iter().enumerate() {
        regions = regions.with_region(
            id.clone(),
            container(text(format!("区域 {}", index + 1)))
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(8),
        );
    }
    workspace_view(
        &state.controller,
        regions,
        ThemeMode::Dark.colors(),
        |action| action,
    )
}

fn list_view(item_count: usize, selected: usize) -> Element<'static, usize, Theme, Renderer> {
    let colors = ThemeMode::Dark.colors();
    let mut items = column![].spacing(4);
    for index in 0..item_count {
        let is_selected = index == selected;
        items = items.push(
            ListItem::label(format!("节点 {}", index + 1))
                .leading(status_indicator(
                    is_selected,
                    10.0,
                    if is_selected {
                        colors.accent
                    } else {
                        colors.faint
                    },
                ))
                .trailing(
                    text(format!("{:04}", index + 1))
                        .size(11)
                        .color(colors.muted),
                )
                .selected(is_selected)
                .on_select(index)
                .view(colors),
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
