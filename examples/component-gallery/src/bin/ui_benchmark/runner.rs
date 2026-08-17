use std::sync::Arc;
use std::time::{Duration, Instant};

use component_gallery::{GalleryMessage, GallerySection, GalleryState};
use iced_wgpu::wgpu;
use iced_winit::futures::futures::executor;
use nana_ui::runtime::{
    ContextMenu, ContextMenuItem, DocumentId, Dropdown, DropdownOption, LayoutViewport, LengthSpec,
    List, ListItem, NodeStyle, RuntimeDocument, ScrollAxes, ScrollView, SearchDropdown,
    SearchDropdownOption, Text, Workspace, WorkspaceRegionSlot,
};
use nana_ui::{
    NanaTextShaper, RegionId, RegionRole, RegionState, RuntimeInputAdapter, ScenePaintViewport,
    SceneWgpuPainter, SettingsTabId, ThemeMode, ThemeModeExt, WorkspaceAction, WorkspaceLayout,
    WorkspaceModel, WorkspaceMutation,
};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};

use crate::report::{AdapterReport, BenchmarkReport, CaseReport, RendererReport, Sample};

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
    let painter = SceneWgpuPainter::new(&device, &queue, format);
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
        queue: &queue,
        painter,
        target: &target,
    };

    let mut cases: Vec<_> = [100, 500, 1_000]
        .into_iter()
        .map(|item_count| benchmark_list_case(item_count, &mut render))
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
    cases.push(benchmark_dropdown_case(&mut render));
    cases.push(benchmark_search_dropdown_case(&mut render));
    cases.push(benchmark_context_menu_case(&mut render));
    for region_count in [20, 50] {
        cases.push(benchmark_workspace_case(region_count, &mut render));
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
        renderer: RendererReport {
            antialiasing: "none",
            target_format: "Bgra8UnormSrgb",
        },
        iterations: ITERATIONS,
        warmup_iterations: WARMUP_ITERATIONS,
        viewport: [VIEWPORT_WIDTH, VIEWPORT_HEIGHT],
        cases,
    }
}

struct RenderContext<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    painter: SceneWgpuPainter,
    target: &'a wgpu::TextureView,
}

fn benchmark_list_case(item_count: usize, render: &mut RenderContext<'_>) -> CaseReport {
    let mut samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        let started = Instant::now();
        let mut document = list_document(item_count, iteration % item_count);
        let view_construction_ms = elapsed_ms(started);
        let sample = measure_document(&mut document, view_construction_ms, render, iteration);
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }
    CaseReport::from_samples(format!("list-{item_count}"), item_count, &samples)
}

fn benchmark_gallery_case(
    name: &str,
    setup: fn(&mut GalleryState, usize),
    render: &mut RenderContext<'_>,
) -> CaseReport {
    let mut state = GalleryState::new();
    resize_gallery(&mut state);
    let mut samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        setup(&mut state, iteration);
        let started = Instant::now();
        state.flush_snapshot_scene();
        let view_construction_ms = elapsed_ms(started);
        let sample = measure_gallery(&mut state, view_construction_ms, render, iteration);
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }
    CaseReport::from_samples(name, 0, &samples)
}

fn benchmark_dropdown_case(render: &mut RenderContext<'_>) -> CaseReport {
    let options = (0..500)
        .map(|index| DropdownOption::new(index.to_string(), format!("选项 {}", index + 1)))
        .collect::<Vec<_>>();
    let mut samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        let started = Instant::now();
        let mut document = dropdown_document(&options, iteration % options.len());
        let view_construction_ms = elapsed_ms(started);
        let sample = measure_document(&mut document, view_construction_ms, render, iteration);
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }
    CaseReport::from_samples("dropdown-500", 500, &samples)
}

fn benchmark_search_dropdown_case(render: &mut RenderContext<'_>) -> CaseReport {
    let options = (0..200)
        .map(|index| {
            SearchDropdownOption::new(index.to_string(), format!("搜索结果 {}", index + 1))
                .hint(format!("分组 {}", index % 8))
        })
        .collect::<Vec<_>>();
    let mut samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        let started = Instant::now();
        let mut document = search_dropdown_document(&options, iteration % options.len());
        let view_construction_ms = elapsed_ms(started);
        let sample = measure_document(&mut document, view_construction_ms, render, iteration);
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }
    CaseReport::from_samples("search-dropdown-200", 200, &samples)
}

fn benchmark_context_menu_case(render: &mut RenderContext<'_>) -> CaseReport {
    let items = (0..12)
        .flat_map(|category| {
            let header =
                ContextMenuItem::new(format!("{category}"), format!("分组 {}", category + 1));
            let children = (0..10).map(move |item| {
                let index = category * 10 + item;
                ContextMenuItem::new(format!("{category}/{item}"), format!("操作 {}", index + 1))
                    .hint(format!("⌘{}", index % 10))
            });
            std::iter::once(header).chain(children)
        })
        .collect::<Vec<_>>();
    let mut samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        let started = Instant::now();
        let mut document = context_menu_document(&items);
        let view_construction_ms = elapsed_ms(started);
        let sample = measure_document(&mut document, view_construction_ms, render, iteration);
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }
    CaseReport::from_samples("context-menu-120", 120, &samples)
}

fn benchmark_workspace_case(region_count: usize, render: &mut RenderContext<'_>) -> CaseReport {
    let mut state = WorkspaceBenchmarkState::new(region_count);
    let mut samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        state.update(iteration);
        let started = Instant::now();
        let mut document = workspace_document(&state);
        let view_construction_ms = elapsed_ms(started);
        let sample = measure_document(&mut document, view_construction_ms, render, iteration);
        if iteration >= WARMUP_ITERATIONS {
            samples.push(sample);
        }
    }
    CaseReport::from_samples(
        format!("workspace-{region_count}-regions"),
        region_count,
        &samples,
    )
}

fn measure_document(
    document: &mut RuntimeDocument,
    view_construction_ms: f64,
    render: &mut RenderContext<'_>,
    iteration: usize,
) -> Sample {
    let viewport = layout_viewport();
    let mut shaper = NanaTextShaper::default();
    let started = Instant::now();
    document
        .flush(viewport, &mut shaper)
        .expect("benchmark Runtime flush");
    let layout_diff_ms = elapsed_ms(started);

    let started = Instant::now();
    dispatch_pointer_sequence(document, iteration);
    document
        .flush(viewport, &mut shaper)
        .expect("benchmark Runtime event flush");
    let event_update_ms = elapsed_ms(started);

    paint_scene(
        document.scene(),
        render,
        iteration,
        view_construction_ms,
        layout_diff_ms,
        event_update_ms,
    )
}

fn measure_gallery(
    state: &mut GalleryState,
    view_construction_ms: f64,
    render: &mut RenderContext<'_>,
    iteration: usize,
) -> Sample {
    let viewport = layout_viewport();
    let mut shaper = NanaTextShaper::default();
    let started = Instant::now();
    if let Some(document) = state.document_mut() {
        document
            .flush(viewport, &mut shaper)
            .expect("benchmark gallery layout flush");
    }
    let layout_diff_ms = elapsed_ms(started);

    let started = Instant::now();
    if let Some(document) = state.document_mut() {
        dispatch_pointer_sequence(document, iteration);
        document
            .flush(viewport, &mut shaper)
            .expect("benchmark gallery event flush");
    }
    let event_update_ms = elapsed_ms(started);

    let scene = state
        .active_scene()
        .expect("gallery benchmark must flush a Runtime scene");
    paint_scene(
        scene,
        render,
        iteration,
        view_construction_ms,
        layout_diff_ms,
        event_update_ms,
    )
}

fn paint_scene(
    scene: &nana_ui::runtime::UiScene,
    render: &mut RenderContext<'_>,
    _iteration: usize,
    view_construction_ms: f64,
    layout_diff_ms: f64,
    event_update_ms: f64,
) -> Sample {
    let background = ThemeMode::Dark.colors().background;
    let mut encoder = render
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nana-ui benchmark paint"),
        });
    let viewport = ScenePaintViewport {
        logical_size: [VIEWPORT_WIDTH as f32, VIEWPORT_HEIGHT as f32],
        physical_size: [VIEWPORT_WIDTH, VIEWPORT_HEIGHT],
        scale_factor: 1.0,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: wgpu_clear([background.r, background.g, background.b, 1.0]),
        clear: true,
    };
    let started = Instant::now();
    render
        .painter
        .paint(scene, &mut encoder, render.target, viewport, None, None)
        .expect("benchmark SceneWgpuPainter must paint");
    let draw_cpu_ms = elapsed_ms(started);

    let started = Instant::now();
    let submission = render.queue.submit([encoder.finish()]);
    render
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .expect("benchmark GPU work must complete");
    let gpu_submit_wait_ms = elapsed_ms(started);

    Sample {
        view_construction_ms,
        layout_diff_ms,
        event_update_ms,
        draw_cpu_ms,
        gpu_submit_wait_ms,
    }
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

fn resize_gallery(state: &mut GalleryState) {
    state.update(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
        width: VIEWPORT_WIDTH as f32,
        height: VIEWPORT_HEIGHT as f32,
    }));
}

fn list_document(item_count: usize, selected: usize) -> RuntimeDocument {
    let document_id = DocumentId::new(1).expect("benchmark document ID is non-zero");
    let mut document = RuntimeDocument::new(document_id);
    document
        .context_mut()
        .set_theme(ThemeMode::Dark)
        .expect("benchmark theme");
    let root = document
        .context_mut()
        .create_component(document_id, List::new().style(fill_column_style(14.0, 8.0)))
        .expect("list root");
    let title = document
        .context_mut()
        .create_detached_component(document_id, Text::new(format!("{item_count} 个节点")))
        .expect("list title");
    let scroll = document
        .context_mut()
        .create_detached_component(
            document_id,
            ScrollView::new(ScrollAxes::Vertical).style(fill_style()),
        )
        .expect("list scroll");
    let items = document
        .context_mut()
        .create_detached_component(document_id, List::new().style(fill_column_style(0.0, 4.0)))
        .expect("list items");
    document
        .context_mut()
        .append_child(root, title)
        .expect("append title");
    document
        .context_mut()
        .append_child(root, scroll)
        .expect("append scroll");
    document
        .context_mut()
        .append_child(scroll, items)
        .expect("append items");
    for index in 0..item_count {
        let item = document
            .context_mut()
            .create_detached_component(
                document_id,
                ListItem::new(format!("节点 {}", index + 1)).selected(index == selected),
            )
            .expect("list item");
        document
            .context_mut()
            .append_child(items, item)
            .expect("append item");
    }
    document
}

fn dropdown_document(options: &[DropdownOption], selected: usize) -> RuntimeDocument {
    let document_id = DocumentId::new(1).expect("benchmark document ID is non-zero");
    let mut document = RuntimeDocument::new(document_id);
    document
        .context_mut()
        .set_theme(ThemeMode::Dark)
        .expect("benchmark theme");
    document
        .context_mut()
        .create_component(
            document_id,
            Dropdown::single(Some(selected.to_string()))
                .options(options.iter().cloned())
                .opened(true),
        )
        .expect("dropdown");
    document
}

fn search_dropdown_document(options: &[SearchDropdownOption], selected: usize) -> RuntimeDocument {
    let document_id = DocumentId::new(1).expect("benchmark document ID is non-zero");
    let mut document = RuntimeDocument::new(document_id);
    document
        .context_mut()
        .set_theme(ThemeMode::Dark)
        .expect("benchmark theme");
    document
        .context_mut()
        .create_component(
            document_id,
            SearchDropdown::new(Some(selected.to_string()))
                .placeholder("搜索")
                .options(options.iter().cloned())
                .opened(true),
        )
        .expect("search dropdown");
    document
}

fn context_menu_document(items: &[ContextMenuItem]) -> RuntimeDocument {
    let document_id = DocumentId::new(1).expect("benchmark document ID is non-zero");
    let mut document = RuntimeDocument::new(document_id);
    document
        .context_mut()
        .set_theme(ThemeMode::Dark)
        .expect("benchmark theme");
    document
        .context_mut()
        .create_component(
            document_id,
            ContextMenu::new(860.0, 160.0)
                .items(items.iter().cloned())
                .searchable(true)
                .query("")
                .active_path(["0"]),
        )
        .expect("context menu");
    document
}

struct WorkspaceBenchmarkState {
    model: WorkspaceModel,
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
        let mut model = WorkspaceModel::with_layout(
            WorkspaceLayout::new(states).expect("benchmark regions are unique"),
        );
        model.update(
            WorkspaceMutation::SetViewport {
                width: VIEWPORT_WIDTH as f32,
                height: VIEWPORT_HEIGHT as f32,
            },
            Duration::ZERO,
        );
        Self { model, region_ids }
    }

    fn update(&mut self, iteration: usize) {
        let resizable = self.region_ids[0].clone();
        self.model
            .update(WorkspaceMutation::ResizeStart(resizable), Duration::ZERO);
        self.model.update(
            WorkspaceMutation::ResizeMove { x: 0.0, y: 0.0 },
            Duration::ZERO,
        );
        self.model.update(
            WorkspaceMutation::ResizeMove {
                x: 140.0 + (iteration % 120) as f32,
                y: 0.0,
            },
            Duration::ZERO,
        );
        self.model
            .update(WorkspaceMutation::ResizeEnd, Duration::ZERO);
        if iteration.is_multiple_of(5) {
            self.model.update(
                WorkspaceMutation::ToggleRegion(self.region_ids[1].clone()),
                Duration::ZERO,
            );
        }
    }
}

fn workspace_document(state: &WorkspaceBenchmarkState) -> RuntimeDocument {
    let document_id = DocumentId::new(1).expect("benchmark document ID is non-zero");
    let mut document = RuntimeDocument::new(document_id);
    document
        .context_mut()
        .set_theme(ThemeMode::Dark)
        .expect("benchmark theme");
    let mut slots = Vec::with_capacity(state.region_ids.len());
    let mut contents = Vec::with_capacity(state.region_ids.len());
    for (index, id) in state.region_ids.iter().enumerate() {
        let content = document
            .context_mut()
            .create_detached_component(document_id, Text::new(format!("区域 {}", index + 1)))
            .expect("workspace region");
        slots.push(WorkspaceRegionSlot::new(id.clone(), content.stable_id()));
        contents.push(content);
    }
    let workspace = document
        .context_mut()
        .create_component(document_id, Workspace::from_model(&state.model, slots))
        .expect("workspace");
    for content in contents {
        document
            .context_mut()
            .append_child(workspace, content)
            .expect("append workspace region");
    }
    document
        .context_mut()
        .assemble_workspace(workspace)
        .expect("assemble workspace");
    document
}

fn dispatch_pointer_sequence(document: &mut RuntimeDocument, iteration: usize) {
    let document_id = document.document();
    let adapter = RuntimeInputAdapter::default();
    let x = 450.0;
    let y = 320.0;
    let direction = if iteration % 20 < 10 { -2.0 } else { 2.0 };
    for event in [
        pointer(PointerPhase::Move, x, y),
        pointer(PointerPhase::Down, x, y),
        pointer(PointerPhase::Up, x, y),
        InputEvent::Wheel {
            x,
            y,
            delta_x: 0.0,
            delta_y: direction,
            line_delta: true,
            modifiers: InputModifiers::default(),
        },
    ] {
        adapter
            .dispatch(document.context_mut(), document_id, &event)
            .expect("benchmark input");
    }
}

fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputEvent {
    InputEvent::Pointer {
        phase,
        pointer_id: 1,
        pointer_type: PointerType::Mouse,
        x,
        y,
        screen_x: x,
        screen_y: y,
        button: 0,
        buttons: u16::from(phase == PointerPhase::Down),
        pressure: 0.5,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: true,
        modifiers: InputModifiers::default(),
    }
}

fn layout_viewport() -> LayoutViewport {
    LayoutViewport::new(VIEWPORT_WIDTH as f32, VIEWPORT_HEIGHT as f32)
}

fn fill_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
    }
    style
}

fn fill_column_style(padding: f32, gap: f32) -> NodeStyle {
    let mut style = fill_style();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.padding_left = Some(LengthSpec::Px(padding));
        layout.padding_right = Some(LengthSpec::Px(padding));
        layout.padding_top = Some(LengthSpec::Px(padding));
        layout.padding_bottom = Some(LengthSpec::Px(padding));
        layout.gap = Some(LengthSpec::Px(gap));
    }
    style
}

fn wgpu_clear([r, g, b, a]: [f32; 4]) -> [f32; 4] {
    [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), a]
}

fn srgb_to_linear(u: f32) -> f32 {
    if u < 0.04045 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}
