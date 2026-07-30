use std::path::PathBuf;

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
    AppTitleBar, Document, GalleryMessage, GalleryState, GalleryTab, LayoutPreset,
    Message as WorkspaceMessage, Navigation, RegionId, SettingsTabId, SurfaceView, ThemeMode,
    WindowChrome, WindowChromeEvent, WindowChromeState, WorkspaceAction, WorkspaceState,
};

use crate::write;

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

    let titlebar_custom_dark_pixels = snapshot_with_cursor(
        &mut renderer,
        titlebar_view(ThemeMode::Dark, WindowChrome::custom()),
        &ThemeMode::Dark.iced_theme(),
        ThemeMode::Dark.colors().background,
        Size::new(900, 120),
        mouse::Cursor::Available(iced::Point::new(880.0, 18.0)),
    );
    let titlebar_custom_dark_path = output.join("titlebar-custom-dark.png");
    write::png(
        &titlebar_custom_dark_path,
        Size::new(900, 120),
        &titlebar_custom_dark_pixels,
    )?;

    let titlebar_custom_light_pixels = snapshot(
        &mut renderer,
        titlebar_view(ThemeMode::Light, WindowChrome::custom()),
        &ThemeMode::Light.iced_theme(),
        ThemeMode::Light.colors().background,
        Size::new(900, 120),
    );
    let titlebar_custom_light_path = output.join("titlebar-custom-light.png");
    write::png(
        &titlebar_custom_light_path,
        Size::new(900, 120),
        &titlebar_custom_light_pixels,
    )?;

    let titlebar_native_leading_pixels = snapshot(
        &mut renderer,
        titlebar_view(ThemeMode::Dark, WindowChrome::native_leading(78.0)),
        &ThemeMode::Dark.iced_theme(),
        ThemeMode::Dark.colors().background,
        Size::new(900, 120),
    );
    let titlebar_native_leading_path = output.join("titlebar-native-leading-dark.png");
    write::png(
        &titlebar_native_leading_path,
        Size::new(900, 120),
        &titlebar_native_leading_pixels,
    )?;

    let workspace = WorkspaceState::new();
    let workspace_pixels = snapshot(
        &mut renderer,
        workspace.view(),
        &workspace.theme_mode().iced_theme(),
        workspace.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_path = output.join("workspace-dark.png");
    write::png(&workspace_path, Size::new(1440, 900), &workspace_pixels)?;

    let workspace_lilia_viewport_pixels = snapshot(
        &mut renderer,
        workspace.view(),
        &workspace.theme_mode().iced_theme(),
        workspace.theme_mode().colors().background,
        Size::new(1280, 720),
    );
    let workspace_lilia_viewport_path = output.join("workspace-lilia-viewport-dark.png");
    write::png(
        &workspace_lilia_viewport_path,
        Size::new(1280, 720),
        &workspace_lilia_viewport_pixels,
    )?;

    let mut workspace_primary_start_edge = WorkspaceState::new();
    workspace_primary_start_edge.update(WorkspaceMessage::Workspace(
        WorkspaceAction::SetRegionCollapsed(RegionId::Resources, true),
    ));
    workspace_primary_start_edge.update(WorkspaceMessage::Workspace(
        WorkspaceAction::AnimationFrame(Instant::now() + iced::time::Duration::from_millis(300)),
    ));
    let workspace_primary_start_edge_pixels = snapshot(
        &mut renderer,
        workspace_primary_start_edge.view(),
        &workspace_primary_start_edge.theme_mode().iced_theme(),
        workspace_primary_start_edge
            .theme_mode()
            .colors()
            .background,
        Size::new(1440, 900),
    );
    let workspace_primary_start_edge_path = output.join("workspace-primary-start-edge-dark.png");
    write::png(
        &workspace_primary_start_edge_path,
        Size::new(1440, 900),
        &workspace_primary_start_edge_pixels,
    )?;

    let mut workspace_primary_corners_disabled = WorkspaceState::new();
    workspace_primary_corners_disabled.update(WorkspaceMessage::SetWorkspaceCorners(false));
    let workspace_primary_corners_disabled_pixels = snapshot(
        &mut renderer,
        workspace_primary_corners_disabled.view(),
        &workspace_primary_corners_disabled.theme_mode().iced_theme(),
        workspace_primary_corners_disabled
            .theme_mode()
            .colors()
            .background,
        Size::new(1440, 900),
    );
    let workspace_primary_corners_disabled_path =
        output.join("workspace-primary-corners-disabled-dark.png");
    write::png(
        &workspace_primary_corners_disabled_path,
        Size::new(1440, 900),
        &workspace_primary_corners_disabled_pixels,
    )?;

    let mut workspace_light = WorkspaceState::new();
    workspace_light.update(WorkspaceMessage::ToggleTheme);
    let workspace_light_pixels = snapshot(
        &mut renderer,
        workspace_light.view(),
        &workspace_light.theme_mode().iced_theme(),
        workspace_light.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_light_path = output.join("workspace-light.png");
    write::png(
        &workspace_light_path,
        Size::new(1440, 900),
        &workspace_light_pixels,
    )?;

    let mut workspace_github = WorkspaceState::new();
    workspace_github.update(WorkspaceMessage::SelectLayout(LayoutPreset::Github));
    let workspace_github_pixels = snapshot(
        &mut renderer,
        workspace_github.view(),
        &workspace_github.theme_mode().iced_theme(),
        workspace_github.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_github_path = output.join("workspace-github-dark.png");
    write::png(
        &workspace_github_path,
        Size::new(1440, 900),
        &workspace_github_pixels,
    )?;

    let mut workspace_live2d = WorkspaceState::new();
    workspace_live2d.update(WorkspaceMessage::SelectLayout(LayoutPreset::Live2D));
    let workspace_live2d_pixels = snapshot(
        &mut renderer,
        workspace_live2d.view(),
        &workspace_live2d.theme_mode().iced_theme(),
        workspace_live2d.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_live2d_path = output.join("workspace-live2d-dark.png");
    write::png(
        &workspace_live2d_path,
        Size::new(1440, 900),
        &workspace_live2d_pixels,
    )?;

    let mut workspace_nodes = WorkspaceState::new();
    workspace_nodes.update(WorkspaceMessage::SelectDocument(Document::Nodes));
    let workspace_nodes_pixels = snapshot(
        &mut renderer,
        workspace_nodes.view(),
        &workspace_nodes.theme_mode().iced_theme(),
        workspace_nodes.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_nodes_path = output.join("workspace-nodes-dark.png");
    write::png(
        &workspace_nodes_path,
        Size::new(1440, 900),
        &workspace_nodes_pixels,
    )?;

    let mut workspace_search = WorkspaceState::new();
    workspace_search.update(WorkspaceMessage::SelectNavigation(Navigation::Search));
    let workspace_search_pixels = snapshot(
        &mut renderer,
        workspace_search.view(),
        &workspace_search.theme_mode().iced_theme(),
        workspace_search.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_search_path = output.join("workspace-search-dark.png");
    write::png(
        &workspace_search_path,
        Size::new(1440, 900),
        &workspace_search_pixels,
    )?;

    let mut workspace_preview = WorkspaceState::new();
    workspace_preview.update(WorkspaceMessage::SelectDocument(Document::Preview));
    let workspace_preview_pixels = snapshot(
        &mut renderer,
        workspace_preview.view(),
        &workspace_preview.theme_mode().iced_theme(),
        workspace_preview.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_preview_path = output.join("workspace-preview-dark.png");
    write::png(
        &workspace_preview_path,
        Size::new(1440, 900),
        &workspace_preview_pixels,
    )?;

    let mut workspace_settings = WorkspaceState::new();
    workspace_settings.update(WorkspaceMessage::SelectNavigation(
        nana_ui::Navigation::Settings,
    ));
    let workspace_settings_pixels = snapshot(
        &mut renderer,
        workspace_settings.view(),
        &workspace_settings.theme_mode().iced_theme(),
        workspace_settings.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_settings_path = output.join("workspace-settings-appearance-dark.png");
    write::png(
        &workspace_settings_path,
        Size::new(1440, 900),
        &workspace_settings_pixels,
    )?;

    let workspace_settings_lilia_viewport_pixels = snapshot(
        &mut renderer,
        workspace_settings.view(),
        &workspace_settings.theme_mode().iced_theme(),
        workspace_settings.theme_mode().colors().background,
        Size::new(1280, 720),
    );
    let workspace_settings_lilia_viewport_path =
        output.join("workspace-settings-lilia-viewport-dark.png");
    write::png(
        &workspace_settings_lilia_viewport_path,
        Size::new(1280, 720),
        &workspace_settings_lilia_viewport_pixels,
    )?;

    let mut workspace_custom_radius = WorkspaceState::new();
    workspace_custom_radius.update(WorkspaceMessage::SetStandardRadius(8));
    let workspace_custom_radius_pixels = snapshot_with_cursor(
        &mut renderer,
        workspace_custom_radius.view(),
        &workspace_custom_radius.theme_mode().iced_theme(),
        workspace_custom_radius.theme_mode().colors().background,
        Size::new(1440, 900),
        mouse::Cursor::Available(iced::Point::new(100.0, 184.0)),
    );
    let workspace_custom_radius_path = output.join("workspace-custom-radius-dark.png");
    write::png(
        &workspace_custom_radius_path,
        Size::new(1440, 900),
        &workspace_custom_radius_pixels,
    )?;

    workspace_settings.update(WorkspaceMessage::SetTheme(nana_ui::ThemeMode::Light));
    let workspace_settings_light_pixels = snapshot(
        &mut renderer,
        workspace_settings.view(),
        &workspace_settings.theme_mode().iced_theme(),
        workspace_settings.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_settings_light_path = output.join("workspace-settings-appearance-light.png");
    write::png(
        &workspace_settings_light_path,
        Size::new(1440, 900),
        &workspace_settings_light_pixels,
    )?;

    workspace_settings.update(WorkspaceMessage::SetTheme(nana_ui::ThemeMode::Dark));
    workspace_settings.update(WorkspaceMessage::SelectSettingsTab(SettingsTabId::from(
        "workspace",
    )));
    let workspace_settings_layout_pixels = snapshot(
        &mut renderer,
        workspace_settings.view(),
        &workspace_settings.theme_mode().iced_theme(),
        workspace_settings.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_settings_layout_path = output.join("workspace-settings-layout-dark.png");
    write::png(
        &workspace_settings_layout_path,
        Size::new(1440, 900),
        &workspace_settings_layout_pixels,
    )?;

    workspace_settings.update(WorkspaceMessage::SelectSettingsTab(SettingsTabId::from(
        "about",
    )));
    let workspace_settings_about_pixels = snapshot(
        &mut renderer,
        workspace_settings.view(),
        &workspace_settings.theme_mode().iced_theme(),
        workspace_settings.theme_mode().colors().background,
        Size::new(1440, 900),
    );
    let workspace_settings_about_path = output.join("workspace-settings-about-dark.png");
    write::png(
        &workspace_settings_about_path,
        Size::new(1440, 900),
        &workspace_settings_about_pixels,
    )?;

    let gallery_controls = GalleryState::new();
    let gallery_controls_pixels = snapshot(
        &mut renderer,
        gallery_controls.view(),
        &gallery_controls.theme_mode().iced_theme(),
        gallery_controls.theme_mode().colors().background,
        Size::new(1180, 760),
    );
    let gallery_controls_path = output.join("component-gallery-controls-dark.png");
    write::png(
        &gallery_controls_path,
        Size::new(1180, 760),
        &gallery_controls_pixels,
    )?;

    let mut gallery_controls_light = GalleryState::new();
    gallery_controls_light.update(GalleryMessage::ToggleTheme);
    let gallery_controls_light_pixels = snapshot(
        &mut renderer,
        gallery_controls_light.view(),
        &gallery_controls_light.theme_mode().iced_theme(),
        gallery_controls_light.theme_mode().colors().background,
        Size::new(1180, 760),
    );
    let gallery_controls_light_path = output.join("component-gallery-controls-light.png");
    write::png(
        &gallery_controls_light_path,
        Size::new(1180, 760),
        &gallery_controls_light_pixels,
    )?;

    let mut gallery_loading = GalleryState::new();
    gallery_loading.update(GalleryMessage::ToggleLoading);
    let gallery_loading_pixels = snapshot(
        &mut renderer,
        gallery_loading.view(),
        &gallery_loading.theme_mode().iced_theme(),
        gallery_loading.theme_mode().colors().background,
        Size::new(1180, 760),
    );
    let gallery_loading_path = output.join("component-gallery-loading-dark.png");
    write::png(
        &gallery_loading_path,
        Size::new(1180, 760),
        &gallery_loading_pixels,
    )?;

    let mut gallery_surfaces = GalleryState::new();
    gallery_surfaces.update(GalleryMessage::SelectTab(GalleryTab::Surfaces));
    let gallery_surfaces_pixels = snapshot(
        &mut renderer,
        gallery_surfaces.view(),
        &gallery_surfaces.theme_mode().iced_theme(),
        gallery_surfaces.theme_mode().colors().background,
        Size::new(1180, 760),
    );
    let gallery_surfaces_path = output.join("component-gallery-surfaces-dark.png");
    write::png(
        &gallery_surfaces_path,
        Size::new(1180, 760),
        &gallery_surfaces_pixels,
    )?;

    let mut gallery_surface_cards = GalleryState::new();
    gallery_surface_cards.update(GalleryMessage::SelectTab(GalleryTab::Surfaces));
    gallery_surface_cards.update(GalleryMessage::SelectSurfaceView(SurfaceView::Nodes));
    let gallery_surface_cards_pixels = snapshot(
        &mut renderer,
        gallery_surface_cards.view(),
        &gallery_surface_cards.theme_mode().iced_theme(),
        gallery_surface_cards.theme_mode().colors().background,
        Size::new(1180, 760),
    );
    let gallery_surface_cards_path = output.join("component-gallery-surface-cards-dark.png");
    write::png(
        &gallery_surface_cards_path,
        Size::new(1180, 760),
        &gallery_surface_cards_pixels,
    )?;

    let mut gallery_context_menu = GalleryState::new();
    gallery_context_menu.update(GalleryMessage::SelectTab(GalleryTab::Feedback));
    gallery_context_menu.update(GalleryMessage::ToggleContextMenu);
    let gallery_context_menu_pixels = snapshot(
        &mut renderer,
        gallery_context_menu.view(),
        &gallery_context_menu.theme_mode().iced_theme(),
        gallery_context_menu.theme_mode().colors().background,
        Size::new(1180, 760),
    );
    let gallery_context_menu_path = output.join("component-gallery-context-menu-dark.png");
    write::png(
        &gallery_context_menu_path,
        Size::new(1180, 760),
        &gallery_context_menu_pixels,
    )?;

    let mut gallery_dialog = GalleryState::new();
    gallery_dialog.update(GalleryMessage::SelectTab(GalleryTab::Feedback));
    gallery_dialog.update(GalleryMessage::ToggleDialog);
    let gallery_dialog_pixels = snapshot(
        &mut renderer,
        gallery_dialog.view(),
        &gallery_dialog.theme_mode().iced_theme(),
        gallery_dialog.theme_mode().colors().background,
        Size::new(1180, 760),
    );
    let gallery_dialog_path = output.join("component-gallery-dialog-dark.png");
    write::png(
        &gallery_dialog_path,
        Size::new(1180, 760),
        &gallery_dialog_pixels,
    )?;

    Ok(vec![
        titlebar_custom_dark_path,
        titlebar_custom_light_path,
        titlebar_native_leading_path,
        workspace_path,
        workspace_lilia_viewport_path,
        workspace_primary_start_edge_path,
        workspace_primary_corners_disabled_path,
        workspace_light_path,
        workspace_github_path,
        workspace_live2d_path,
        workspace_nodes_path,
        workspace_search_path,
        workspace_preview_path,
        workspace_settings_path,
        workspace_settings_lilia_viewport_path,
        workspace_custom_radius_path,
        workspace_settings_light_path,
        workspace_settings_layout_path,
        workspace_settings_about_path,
        gallery_controls_path,
        gallery_controls_light_path,
        gallery_loading_path,
        gallery_surfaces_path,
        gallery_surface_cards_path,
        gallery_context_menu_path,
        gallery_dialog_path,
    ])
}

fn titlebar_view(
    theme: ThemeMode,
    chrome: WindowChrome,
) -> Element<'static, WindowChromeEvent, Theme, Renderer> {
    let colors = theme.colors();
    let state = WindowChromeState::new(chrome);
    let titlebar = AppTitleBar::new("NanaUI", colors)
        .leading(text("NANA").size(12).color(colors.accent))
        .trailing(text("预览").size(11).color(colors.muted))
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
