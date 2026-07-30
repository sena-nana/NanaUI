use std::path::PathBuf;

use iced::{Color, Element, Font, Pixels, Size, Theme, mouse};
use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::core::renderer;
use iced_winit::futures::futures::executor;
use iced_winit::runtime::UserInterface;
use iced_winit::runtime::user_interface;
use nana_ui::{
    GalleryMessage, GalleryState, GalleryTab, LayoutPreset, Message as WorkspaceMessage,
    SurfaceView, WorkspaceState,
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
            default_font: Font::DEFAULT,
            default_text_size: Pixels::from(13),
            metrics_hinting: true,
        },
    );
    let output = std::env::current_dir()?.join("target/ui-snapshots");

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
        workspace_path,
        workspace_light_path,
        workspace_github_path,
        workspace_live2d_path,
        gallery_controls_path,
        gallery_controls_light_path,
        gallery_loading_path,
        gallery_surfaces_path,
        gallery_surface_cards_path,
        gallery_context_menu_path,
        gallery_dialog_path,
    ])
}

fn snapshot<Message>(
    renderer: &mut Renderer,
    view: Element<'_, Message, Theme, Renderer>,
    theme: &Theme,
    background: Color,
    size: Size<u32>,
) -> Vec<u8> {
    let viewport = Viewport::with_physical_size(size, renderer::Scale::default());
    let mut interface = UserInterface::build(
        view,
        viewport.logical_size(),
        user_interface::Cache::new(),
        renderer,
    );
    interface.draw(
        renderer,
        theme,
        &renderer::Style {
            text_color: theme.palette().background.base.text,
        },
        mouse::Cursor::Unavailable,
    );
    let cache = interface.into_cache();
    let pixels = renderer.screenshot(&viewport, background);
    drop(cache);
    pixels
}
