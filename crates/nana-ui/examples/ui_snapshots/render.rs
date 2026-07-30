use std::path::{Path, PathBuf};

use iced::widget::{column, container, space, text};
use iced::{Color, Element, Length, Pixels, Size, Theme, font, mouse};
use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::core::time::Instant;
use iced_winit::core::{Event, renderer, shell, window};
use iced_winit::futures::futures::executor;
use iced_winit::runtime::UserInterface;
use iced_winit::runtime::user_interface;
use nana_ui::{
    AppTitleBar, GalleryMessage, GallerySection, GalleryState, RegionId, SettingsTabId,
    SurfaceView, ThemeMode, WindowChrome, WindowChromeEvent, WindowChromeState, WorkspaceAction,
};

use crate::write;

const GALLERY_SIZE: Size<u32> = Size::new(1280, 800);

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
    let engine = Engine::new(&adapter, device, queue, format, None, Shell::headless());
    let mut renderer = Renderer::new(
        engine,
        renderer::Settings {
            default_font: nana_ui::ui_font(font::Weight::Normal),
            default_text_size: Pixels::from(13),
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
    ];

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

    let mut cards = GalleryState::new();
    cards.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    cards.update(GalleryMessage::SelectSurfaceView(SurfaceView::Cards));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-cards-dark.png",
        &cards,
    )?);

    let mut feedback = GalleryState::new();
    feedback.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-feedback-dark.png",
        &feedback,
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

    let mut workspace = GalleryState::new();
    workspace.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-workspace-dark.png",
        &workspace,
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
