use std::path::{Path, PathBuf};

use component_gallery::{GalleryMessage, GallerySection, GalleryState, SurfaceView};
use iced::widget::{column, container, space, text};
use iced::{Color, Element, Length, Pixels, Point, Size, Theme, font, mouse};
use iced_wgpu::graphics::{Antialiasing, Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::core::time::Instant;
use iced_winit::core::{Event, renderer, shell, window};
use iced_winit::futures::futures::executor;
use iced_winit::runtime::UserInterface;
use iced_winit::runtime::user_interface;
use nana_ui::{
    AppTitleBar, DockAction, DockBounds, DockChromeStyle, DockContents, DockController,
    DockDropZone, DockHostEffect, DockId, DockItemSpec, DockLayout, DockNode, DockSurfaceId,
    FloatingDock, RegionId, SettingsTabId, ThemeMode, UI_BASE_TEXT_SIZE, WindowChrome,
    WindowChromeEvent, WindowChromeState, WorkspaceAction, dock_window_workspace, dock_workspace,
};

use crate::write;

const GALLERY_SIZE: Size<u32> = Size::new(1280, 800);

#[derive(Clone, Copy)]
enum DockPreviewPhase {
    Candidate,
    Transition,
    Settled,
    Retarget,
}

pub fn generate() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let (device, queue) = executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nana-ui snapshot device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))?;
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let engine = Engine::new(
        &adapter,
        device,
        queue,
        format,
        Some(Antialiasing::MSAAx4),
        Shell::headless(),
    );
    let mut renderer = Renderer::new(
        engine,
        renderer::Settings {
            default_font: nana_ui::ui_font(font::Weight::Normal),
            default_text_size: Pixels::from(UI_BASE_TEXT_SIZE),
            metrics_hinting: true,
        },
    );
    let mut font_system = iced_wgpu::graphics::text::font_system()
        .write()
        .expect("font system");
    for source in nana_ui::ui_font_sources() {
        font_system.load_font(std::borrow::Cow::Borrowed(source));
    }
    drop(font_system);

    let output = std::env::current_dir()?.join("target/ui-snapshots");
    let mut paths = vec![
        titlebar_snapshot(
            &mut renderer,
            &output,
            "titlebar-custom-dark.png",
            ThemeMode::Dark,
            WindowChrome::custom(),
            mouse::Cursor::Available(iced::Point::new(880.0, 18.0)),
        )?,
        titlebar_snapshot(
            &mut renderer,
            &output,
            "titlebar-custom-light.png",
            ThemeMode::Light,
            WindowChrome::custom(),
            mouse::Cursor::Unavailable,
        )?,
        titlebar_snapshot(
            &mut renderer,
            &output,
            "titlebar-native-leading-dark.png",
            ThemeMode::Dark,
            WindowChrome::native_leading(78.0),
            mouse::Cursor::Unavailable,
        )?,
        dock_window_snapshot(
            &mut renderer,
            &output,
            "dock-window-custom-dark.png",
            ThemeMode::Dark,
            WindowChrome::custom(),
        )?,
        dock_window_snapshot(
            &mut renderer,
            &output,
            "dock-window-native-leading-light.png",
            ThemeMode::Light,
            WindowChrome::native_leading(78.0),
        )?,
    ];

    for (suffix, theme) in [("dark", ThemeMode::Dark), ("light", ThemeMode::Light)] {
        paths.push(dock_window_merged_snapshot(
            &mut renderer,
            &output,
            &format!("dock-window-merged-tabs-{suffix}.png"),
            theme,
            WindowChrome::custom(),
            DockNode::tabs(
                [
                    DockId::from("scenes"),
                    DockId::from("mixer"),
                    DockId::from("controls"),
                ],
                "mixer",
            ),
        )?);
        paths.push(dock_window_merged_snapshot(
            &mut renderer,
            &output,
            &format!("dock-window-merged-split-{suffix}.png"),
            theme,
            WindowChrome::custom(),
            DockNode::split(
                nana_ui::DockAxis::Horizontal,
                0.5,
                DockNode::tabs([DockId::from("scenes"), DockId::from("mixer")], "scenes"),
                DockNode::item("controls"),
            ),
        )?);
        paths.push(dock_drag_window_snapshot(
            &mut renderer,
            &output,
            &format!("dock-drag-window-{suffix}.png"),
            theme,
            WindowChrome::custom(),
        )?);
        for (name, zone) in [
            ("left", DockDropZone::Left),
            ("right", DockDropZone::Right),
            ("top", DockDropZone::Top),
            ("bottom", DockDropZone::Bottom),
            ("tab", DockDropZone::Tab),
        ] {
            paths.push(dock_preview_snapshot(
                &mut renderer,
                &output,
                &format!("dock-preview-{name}-{suffix}.png"),
                theme,
                zone,
                DockPreviewPhase::Settled,
            )?);
        }
        paths.push(dock_preview_snapshot(
            &mut renderer,
            &output,
            &format!("dock-preview-tab-transition-{suffix}.png"),
            theme,
            DockDropZone::Tab,
            DockPreviewPhase::Transition,
        )?);
        paths.push(dock_preview_snapshot(
            &mut renderer,
            &output,
            &format!("dock-preview-retarget-tab-{suffix}.png"),
            theme,
            DockDropZone::Left,
            DockPreviewPhase::Retarget,
        )?);
        paths.push(dock_preview_snapshot(
            &mut renderer,
            &output,
            &format!("dock-hover-left-{suffix}.png"),
            theme,
            DockDropZone::Left,
            DockPreviewPhase::Candidate,
        )?);
        paths.push(dock_outside_snapshot(
            &mut renderer,
            &output,
            &format!("dock-preview-outside-{suffix}.png"),
            theme,
        )?);
    }

    let controls = GalleryState::new();
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-controls-dark.png",
        &controls,
    )?);

    let mut controls_light = GalleryState::new();
    controls_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-controls-light.png",
        &controls_light,
    )?);

    let mut loading = GalleryState::new();
    loading.update(GalleryMessage::ToggleLoading);
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-loading-dark.png",
        &loading,
    )?);

    let mut surfaces = GalleryState::new();
    surfaces.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-surfaces-dark.png",
        &surfaces,
    )?);

    let mut surfaces_light = GalleryState::new();
    surfaces_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    surfaces_light.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-surfaces-light.png",
        &surfaces_light,
    )?);

    let mut cards = GalleryState::new();
    cards.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    cards.update(GalleryMessage::SelectSurfaceView(SurfaceView::Cards));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-cards-dark.png",
        &cards,
    )?);

    surfaces_light.update(GalleryMessage::SelectSurfaceView(SurfaceView::Cards));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-cards-light.png",
        &surfaces_light,
    )?);

    let mut feedback = GalleryState::new();
    feedback.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-feedback-dark.png",
        &feedback,
    )?);

    let mut popover = GalleryState::new();
    popover.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    popover.update(GalleryMessage::TogglePopover);
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-popover-dark.png",
        &popover,
    )?);

    let mut context_menu = GalleryState::new();
    context_menu.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    context_menu.update(GalleryMessage::ToggleContextMenu);
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-context-menu-dark.png",
        &context_menu,
    )?);

    let mut dialog = GalleryState::new();
    dialog.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    dialog.update(GalleryMessage::ToggleDialog);
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-dialog-dark.png",
        &dialog,
    )?);

    let mut image_viewer = GalleryState::new();
    image_viewer.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    image_viewer.update(GalleryMessage::ToggleImageViewer);
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-image-viewer-dark.png",
        &image_viewer,
    )?);

    let mut workspace = GalleryState::new();
    workspace.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-workspace-dark.png",
        &workspace,
    )?);

    let mut workspace_dock_preview = GalleryState::new();
    workspace_dock_preview.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    prepare_dock_preview(&mut workspace_dock_preview);
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-workspace-dock-preview-dark.png",
        &workspace_dock_preview,
    )?);
    let mut workspace_dock_preview_light = GalleryState::new();
    workspace_dock_preview_light.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    workspace_dock_preview_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    prepare_dock_preview(&mut workspace_dock_preview_light);
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-workspace-dock-preview-light.png",
        &workspace_dock_preview_light,
    )?);

    let mut sidebar_collapsed = GalleryState::new();
    sidebar_collapsed.update(GalleryMessage::Workspace(
        WorkspaceAction::SetRegionCollapsed(RegionId::Resources, true),
    ));
    sidebar_collapsed.update(GalleryMessage::Workspace(WorkspaceAction::AnimationFrame(
        Instant::now() + iced::time::Duration::from_millis(300),
    )));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-sidebar-collapsed-dark.png",
        &sidebar_collapsed,
    )?);

    let mut settings = GalleryState::new();
    settings.update(GalleryMessage::OpenSettings);
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-settings-appearance-dark.png",
        &settings,
    )?);

    settings.update(GalleryMessage::SetTheme(ThemeMode::Light));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-settings-appearance-light.png",
        &settings,
    )?);

    settings.update(GalleryMessage::SetTheme(ThemeMode::Dark));
    settings.update(GalleryMessage::SelectSettingsTab(SettingsTabId::from(
        "workspace",
    )));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-settings-workspace-dark.png",
        &settings,
    )?);

    settings.update(GalleryMessage::SelectSettingsTab(SettingsTabId::from(
        "about",
    )));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-settings-about-dark.png",
        &settings,
    )?);

    Ok(paths)
}

fn titlebar_snapshot(
    renderer: &mut Renderer,
    output: &Path,
    name: &str,
    theme: ThemeMode,
    chrome: WindowChrome,
    cursor: mouse::Cursor,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(900, 120);
    let pixels = snapshot_with_cursor(
        renderer,
        titlebar_view(theme, chrome),
        &theme.iced_theme(),
        theme.colors().background,
        size,
        cursor,
    );
    let path = output.join(name);
    write::png(&path, size, &pixels)?;
    Ok(path)
}

fn dock_window_snapshot(
    renderer: &mut Renderer,
    output: &Path,
    name: &str,
    theme: ThemeMode,
    chrome: WindowChrome,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(420, 320);
    let surface = DockSurfaceId(1);
    let mut layout = DockLayout::new(DockNode::item("editor"));
    layout.floating.push(FloatingDock {
        surface,
        root: DockNode::item("scenes"),
        bounds: DockBounds::new(120.0, 120.0, size.width as f32, size.height as f32),
        monitor: None,
    });
    let mut controller = DockController::new(
        "editor",
        [
            DockItemSpec::new("editor", "Editor").closeable(false),
            DockItemSpec::new("scenes", "场景"),
        ],
        layout,
    )?;
    controller.set_chrome_style(DockChromeStyle::Card);
    let window_chrome = WindowChromeState::new(chrome);
    let colors = theme.colors();
    let view = dock_window_workspace(
        &controller,
        surface,
        DockContents::new().insert(
            "scenes",
            container(
                column![
                    text("Scene A").size(13).color(colors.text),
                    text("Preview 当前场景").size(11).color(colors.muted),
                ]
                .spacing(6),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        ),
        &window_chrome,
        |_| (),
        |_| (),
        colors,
    );
    let pixels = snapshot(renderer, view, &theme.iced_theme(), colors.background, size);
    let path = output.join(name);
    write::png(&path, size, &pixels)?;
    Ok(path)
}

fn dock_window_merged_snapshot(
    renderer: &mut Renderer,
    output: &Path,
    name: &str,
    theme: ThemeMode,
    chrome: WindowChrome,
    root: DockNode,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(520, 360);
    let surface = DockSurfaceId(1);
    let mut layout = DockLayout::new(DockNode::item("editor"));
    layout.floating.push(FloatingDock {
        surface,
        root,
        bounds: DockBounds::new(120.0, 120.0, size.width as f32, size.height as f32),
        monitor: None,
    });
    let mut controller = DockController::new(
        "editor",
        [
            DockItemSpec::new("editor", "Editor").closeable(false),
            DockItemSpec::new("scenes", "场景"),
            DockItemSpec::new("mixer", "混音"),
            DockItemSpec::new("controls", "控制"),
        ],
        layout,
    )?;
    controller.set_chrome_style(DockChromeStyle::Card);
    let window_chrome = WindowChromeState::new(chrome);
    let colors = theme.colors();
    let contents = DockContents::new()
        .insert(
            "scenes",
            container(
                column![
                    text("Scene A").size(13).color(colors.text),
                    text("场景列表").size(11).color(colors.muted),
                ]
                .spacing(6),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .insert(
            "mixer",
            container(
                column![
                    text("Program Bus").size(13).color(colors.text),
                    text("音频与节目输出").size(11).color(colors.muted),
                ]
                .spacing(6),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .insert(
            "controls",
            container(
                column![
                    text("Cue Controls").size(13).color(colors.text),
                    text("导播控制").size(11).color(colors.muted),
                ]
                .spacing(6),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        );
    let view = dock_window_workspace(
        &controller,
        surface,
        contents,
        &window_chrome,
        |_| (),
        |_| (),
        colors,
    );
    let pixels = snapshot(renderer, view, &theme.iced_theme(), colors.background, size);
    let path = output.join(name);
    write::png(&path, size, &pixels)?;
    Ok(path)
}

fn dock_drag_window_snapshot(
    renderer: &mut Renderer,
    output: &Path,
    name: &str,
    theme: ThemeMode,
    chrome: WindowChrome,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(420, 240);
    let surface = DockSurfaceId(0);
    let mut controller = DockController::new(
        "editor",
        [
            DockItemSpec::new("editor", "Editor").closeable(false),
            DockItemSpec::new("scenes", "场景"),
        ],
        DockLayout::new(DockNode::split(
            nana_ui::DockAxis::Horizontal,
            0.5,
            DockNode::item("scenes"),
            DockNode::item("editor"),
        )),
    )?;
    controller.update(DockAction::SurfaceGeometry {
        surface,
        bounds: DockBounds::new(0.0, 0.0, size.width as f32, size.height as f32),
    });
    controller.update(DockAction::DragStart {
        surface,
        id: DockId::from("scenes"),
    });
    controller.update(DockAction::DragMove {
        surface,
        position: Point::new(0.0, 0.0),
    });
    let opened = controller.update(DockAction::DragMove {
        surface,
        position: Point::new(60.0, 120.0),
    });
    let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
        return Err("dock drag preview surface was not opened".into());
    };
    let window_chrome = WindowChromeState::new(chrome);
    let colors = theme.colors();
    let view = dock_window_workspace(
        &controller,
        floating.surface,
        DockContents::new(),
        &window_chrome,
        |_| (),
        |_| (),
        colors,
    );
    let pixels = snapshot(renderer, view, &theme.iced_theme(), colors.background, size);
    let path = output.join(name);
    write::png(&path, size, &pixels)?;
    Ok(path)
}

fn dock_preview_snapshot(
    renderer: &mut Renderer,
    output: &Path,
    name: &str,
    theme: ThemeMode,
    zone: DockDropZone,
    phase: DockPreviewPhase,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(420, 240);
    let position = match zone {
        DockDropZone::Left => Point::new(20.0, 160.0),
        DockDropZone::Right => Point::new(400.0, 160.0),
        DockDropZone::Top => Point::new(200.0, 20.0),
        DockDropZone::Bottom => Point::new(200.0, 100.0),
        DockDropZone::Tab => Point::new(200.0, 50.0),
    };
    let controller = dock_preview_controller(size, position, phase)?;
    let colors = theme.colors();
    let view = dock_workspace(
        &controller,
        DockSurfaceId(0),
        dock_preview_contents(),
        |_| (),
        colors,
    );
    let pixels = snapshot(renderer, view, &theme.iced_theme(), colors.background, size);
    let path = output.join(name);
    write::png(&path, size, &pixels)?;
    Ok(path)
}

fn dock_outside_snapshot(
    renderer: &mut Renderer,
    output: &Path,
    name: &str,
    theme: ThemeMode,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(420, 240);
    let controller =
        dock_preview_controller(size, Point::new(410.0, 80.0), DockPreviewPhase::Candidate)?;
    let colors = theme.colors();
    let view = dock_workspace(
        &controller,
        DockSurfaceId(0),
        dock_preview_contents(),
        |_| (),
        colors,
    );
    let pixels = snapshot(renderer, view, &theme.iced_theme(), colors.background, size);
    let path = output.join(name);
    write::png(&path, size, &pixels)?;
    Ok(path)
}

fn dock_preview_controller(
    size: Size<u32>,
    position: Point,
    phase: DockPreviewPhase,
) -> Result<DockController, Box<dyn std::error::Error>> {
    let mut controller = DockController::new(
        "editor",
        [
            DockItemSpec::new("editor", "Editor").closeable(false),
            DockItemSpec::new("source", "Source"),
            DockItemSpec::new("panel", "Panel"),
        ],
        DockLayout::new(DockNode::split(
            nana_ui::DockAxis::Horizontal,
            0.5,
            DockNode::item("source"),
            DockNode::split(
                nana_ui::DockAxis::Vertical,
                0.5,
                DockNode::item("panel"),
                DockNode::item("editor"),
            ),
        )),
    )?;
    controller.set_chrome_style(DockChromeStyle::Card);
    controller.update(DockAction::SurfaceGeometry {
        surface: DockSurfaceId(0),
        bounds: DockBounds::new(0.0, 0.0, size.width as f32, size.height as f32),
    });
    controller.update(DockAction::DragStart {
        surface: DockSurfaceId(0),
        id: DockId::from("source"),
    });
    controller.update(DockAction::DragMove {
        surface: DockSurfaceId(0),
        position: Point::new(0.0, 0.0),
    });
    controller.update(DockAction::DragMove {
        surface: DockSurfaceId(0),
        position,
    });
    match phase {
        DockPreviewPhase::Candidate => {}
        DockPreviewPhase::Transition => {
            std::thread::sleep(std::time::Duration::from_millis(350));
            controller.update(DockAction::Hover(false));
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        DockPreviewPhase::Settled => {
            std::thread::sleep(std::time::Duration::from_millis(350));
            controller.update(DockAction::Hover(false));
            std::thread::sleep(std::time::Duration::from_millis(100));
            controller.update(DockAction::Hover(false));
        }
        DockPreviewPhase::Retarget => {
            std::thread::sleep(std::time::Duration::from_millis(350));
            controller.update(DockAction::Hover(false));
            std::thread::sleep(std::time::Duration::from_millis(60));
            controller.update(DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(200.0, 180.0),
            });
            std::thread::sleep(std::time::Duration::from_millis(350));
            controller.update(DockAction::Hover(false));
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
    }
    Ok(controller)
}

fn dock_preview_contents() -> DockContents<'static, ()> {
    DockContents::new()
        .insert("source", container(text("Source")).center(Length::Fill))
        .insert("panel", container(text("Panel")).center(Length::Fill))
        .insert("editor", container(text("Editor")).center(Length::Fill))
}

fn gallery_snapshot(
    renderer: &mut Renderer,
    output: &Path,
    name: &str,
    state: &GalleryState,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let pixels = snapshot(
        renderer,
        state.view(),
        &state.theme_mode().iced_theme(),
        state.theme_mode().colors().background,
        GALLERY_SIZE,
    );
    let path = output.join(name);
    write::png(&path, GALLERY_SIZE, &pixels)?;
    Ok(path)
}

fn prepare_dock_preview(state: &mut GalleryState) {
    let surface = DockSurfaceId(0);
    state.update(GalleryMessage::Dock(DockAction::DragStart {
        surface,
        id: DockId::from("gallery.sources"),
    }));
    state.update(GalleryMessage::Dock(DockAction::DragMove {
        surface,
        position: Point::new(350.0, 250.0),
    }));
    state.update(GalleryMessage::Dock(DockAction::DragMove {
        surface,
        position: Point::new(355.0, 250.0),
    }));
    std::thread::sleep(std::time::Duration::from_millis(350));
    state.update(GalleryMessage::Dock(DockAction::Hover(false)));
    std::thread::sleep(std::time::Duration::from_millis(100));
    state.update(GalleryMessage::Dock(DockAction::Hover(false)));
}

fn titlebar_view(
    theme: ThemeMode,
    chrome: WindowChrome,
) -> Element<'static, WindowChromeEvent, Theme, Renderer> {
    let colors = theme.colors();
    let state = WindowChromeState::new(chrome);
    let titlebar = AppTitleBar::new("NanaUI", colors)
        .leading(text("NANA").size(12).color(colors.accent))
        .trailing(text("Gallery").size(11).color(colors.muted))
        .window_chrome(&state, |event| event)
        .view();

    container(column![titlebar, space().height(Length::Fill)])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(colors.background)
                .color(colors.text)
        })
        .into()
}

fn snapshot<Message>(
    renderer: &mut Renderer,
    view: Element<'_, Message, Theme, Renderer>,
    theme: &Theme,
    background: Color,
    size: Size<u32>,
) -> Vec<u8> {
    snapshot_with_cursor(
        renderer,
        view,
        theme,
        background,
        size,
        mouse::Cursor::Unavailable,
    )
}

fn snapshot_with_cursor<Message>(
    renderer: &mut Renderer,
    view: Element<'_, Message, Theme, Renderer>,
    theme: &Theme,
    background: Color,
    size: Size<u32>,
    cursor: mouse::Cursor,
) -> Vec<u8> {
    let viewport = Viewport::with_physical_size(size, renderer::Scale::default());
    let mut interface = UserInterface::build(
        view,
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
        cursor,
        renderer,
        &mut Vec::new(),
    );
    interface.draw(
        renderer,
        theme,
        &renderer::Style {
            text_color: theme.palette().background.base.text,
        },
        cursor,
    );
    let cache = interface.into_cache();
    let pixels = renderer.screenshot(&viewport, background);
    drop(cache);
    pixels
}
