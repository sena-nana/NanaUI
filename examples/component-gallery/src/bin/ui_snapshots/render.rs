use std::path::{Path, PathBuf};

use component_gallery::{GalleryMessage, GallerySection, GalleryState, SurfaceView};
use iced::widget::{column, container, row, space, text};
use iced::{Alignment, Color, Element, Length, Pixels, Point, Size, Theme, font, mouse};
use iced_wgpu::graphics::{Antialiasing, Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::core::time::Instant;
use iced_winit::core::{Event, renderer, shell, window};
use iced_winit::futures::futures::executor;
use iced_winit::runtime::UserInterface;
use iced_winit::runtime::user_interface;
use nana_ui::compatibility::{
    Button as LegacyButton, Checkbox as LegacyCheckbox, Input as LegacyInput,
};
use nana_ui::runtime::{
    AppContext, Button as RuntimeButton, Card as RuntimeCard, Checkbox as RuntimeCheckbox,
    DocumentId, IconButton as RuntimeIconButton, LayoutBox, List as RuntimeList,
    ListItem as RuntimeListItem, MutationQueue, NodeStyle, ScrollAxes, ScrollOffset,
    ScrollView as RuntimeScrollView, Slider as RuntimeSlider, Switch as RuntimeSwitch,
    Tab as RuntimeTab, TabList as RuntimeTabList, Table as RuntimeTable,
    TableCell as RuntimeTableCell, TableRow as RuntimeTableRow, Text as RuntimeText,
    TextArea as RuntimeTextArea, TextInput as RuntimeTextInput, TextVerticalAlignment,
};
use nana_ui::{
    AppTitleBar, ButtonKind, DockAction, DockBounds, DockChromeStyle, DockContents, DockController,
    DockDropZone, DockHostEffect, DockId, DockItemSpec, DockLayout, DockNode, DockSurfaceId,
    FloatingDock, LayoutBounds, LayoutProbe, PaneChromeActionKind, RegionId, SettingsTabId,
    ThemeMode, ThemeModeExt, UI_BASE_TEXT_SIZE, WindowChrome, WindowChromeEvent, WindowChromeState,
    WorkspaceAction, dock_window_workspace, dock_workspace,
};
use nana_ui::{CommandPaletteEvent, ContextMenuEvent};
use nana_ui_core::{LayoutStyle, SemanticColorRole};
use nana_ui_scene::UiScene;

use crate::write;

#[path = "render/migration_next.rs"]
mod migration_next;

const GALLERY_SIZE: Size<u32> = Size::new(1280, 800);

#[derive(Clone, Copy)]
enum DockPreviewPhase {
    Candidate,
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

    let output = std::env::var_os("NANA_UI_SNAPSHOT_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?.join("target/ui-snapshots"));
    let mut paths = vec![
        runtime_scene_snapshot(
            &mut renderer,
            &output,
            "runtime-scene-dark.png",
            ThemeMode::Dark,
        )?,
        runtime_scene_snapshot(
            &mut renderer,
            &output,
            "runtime-scene-light.png",
            ThemeMode::Light,
        )?,
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
    paths.extend(component_migration_snapshots(
        &mut renderer,
        &output,
        ThemeMode::Dark,
    )?);
    for theme in [ThemeMode::Dark, ThemeMode::Light] {
        paths.extend(migration_next::generate(&mut renderer, &output, theme)?);
    }

    for (suffix, theme) in [("dark", ThemeMode::Dark), ("light", ThemeMode::Light)] {
        paths.push(dock_window_merged_snapshot(
            &mut renderer,
            &output,
            &format!("dock-window-merged-tabs-{suffix}.png"),
            theme,
            WindowChrome::custom(),
            DockNode::tabs(
                [
                    DockId::from("navigation"),
                    DockId::from("console"),
                    DockId::from("output"),
                ],
                "console",
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
                DockNode::tabs(
                    [DockId::from("navigation"), DockId::from("console")],
                    "navigation",
                ),
                DockNode::item("output"),
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
    paths.push(gallery_snapshot_with_cursor(
        &mut renderer,
        &output,
        "gallery-sidebar-tools-dark.png",
        &controls,
        mouse::Cursor::Available(Point::new(180.0, 60.0)),
    )?);
    paths.push(gallery_snapshot_with_cursor(
        &mut renderer,
        &output,
        "gallery-sidebar-tools-light.png",
        &controls_light,
        mouse::Cursor::Available(Point::new(180.0, 60.0)),
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

    surfaces.update(GalleryMessage::PaneChrome(
        PaneChromeActionKind::SplitHorizontal,
    ));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-surfaces-split-dark.png",
        &surfaces,
    )?);

    surfaces_light.update(GalleryMessage::PaneChrome(
        PaneChromeActionKind::SplitHorizontal,
    ));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-surfaces-split-light.png",
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

    let mut rich_text = GalleryState::new();
    rich_text.update(GalleryMessage::SelectSection(GallerySection::RichText));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-rich-text-dark.png",
        &rich_text,
    )?);
    rich_text.update(GalleryMessage::SetTheme(ThemeMode::Light));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-rich-text-light.png",
        &rich_text,
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
    context_menu.update(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
        width: GALLERY_SIZE.width as f32,
        height: GALLERY_SIZE.height as f32,
    }));
    context_menu.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    context_menu.update(GalleryMessage::ToggleContextMenu);
    context_menu.update(GalleryMessage::ContextMenu(ContextMenuEvent::OpenSubmenu(
        vec![0],
    )));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-context-menu-dark.png",
        &context_menu,
    )?);
    context_menu.update(GalleryMessage::SetTheme(ThemeMode::Light));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-context-menu-light.png",
        &context_menu,
    )?);

    let mut context_menu_search = GalleryState::new();
    context_menu_search.update(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
        width: GALLERY_SIZE.width as f32,
        height: GALLERY_SIZE.height as f32,
    }));
    context_menu_search.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    context_menu_search.update(GalleryMessage::ToggleContextMenu);
    context_menu_search.update(GalleryMessage::ContextMenu(ContextMenuEvent::Search(
        "重命名".to_owned(),
    )));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-context-menu-search-dark.png",
        &context_menu_search,
    )?);

    let mut context_menu_search_light = GalleryState::new();
    context_menu_search_light.update(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
        width: GALLERY_SIZE.width as f32,
        height: GALLERY_SIZE.height as f32,
    }));
    context_menu_search_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    context_menu_search_light.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    context_menu_search_light.update(GalleryMessage::ToggleContextMenu);
    context_menu_search_light.update(GalleryMessage::ContextMenu(ContextMenuEvent::Search(
        "copy".to_owned(),
    )));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-context-menu-search-light.png",
        &context_menu_search_light,
    )?);

    let mut command_palette = GalleryState::new();
    command_palette.update(GalleryMessage::ToggleCommandPalette);
    command_palette.update(GalleryMessage::CommandPalette(CommandPaletteEvent::Search(
        "工作区".to_owned(),
    )));
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-command-palette-dark.png",
        &command_palette,
    )?);

    let mut command_palette_light = GalleryState::new();
    command_palette_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    command_palette_light.update(GalleryMessage::ToggleCommandPalette);
    paths.push(gallery_snapshot(
        &mut renderer,
        &output,
        "gallery-command-palette-light.png",
        &command_palette_light,
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
        std::time::Duration::from_millis(300),
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

const MIGRATION_SIZE: Size<u32> = Size::new(520, 220);

#[derive(Debug, Clone, Copy)]
struct MigrationLayoutMessage {
    component: &'static str,
    bounds: LayoutBounds,
}

fn component_migration_snapshots(
    renderer: &mut Renderer,
    output: &Path,
    theme: ThemeMode,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let legacy = migration_legacy_view(theme);
    let (legacy_pixels, legacy_layout) = snapshot_with_messages(
        renderer,
        legacy,
        &theme.iced_theme(),
        theme.colors().background,
        MIGRATION_SIZE,
    );
    let legacy_path = output.join("migration-first-batch-iced-dark.png");
    write::png(&legacy_path, MIGRATION_SIZE, &legacy_pixels)?;

    let (runtime_scene, runtime_layout) = migration_runtime_scene(theme)?;
    let runtime_view = nana_ui::IcedSceneView::new(
        &runtime_scene,
        Size::new(MIGRATION_SIZE.width as f32, MIGRATION_SIZE.height as f32),
    )?;
    let runtime_view: Element<'_, (), Theme, Renderer> = runtime_view.into();
    let runtime_pixels = snapshot(
        renderer,
        runtime_view,
        &theme.iced_theme(),
        theme.colors().background,
        MIGRATION_SIZE,
    );
    let runtime_path = output.join("migration-first-batch-runtime-dark.png");
    write::png(&runtime_path, MIGRATION_SIZE, &runtime_pixels)?;

    let comparison_size = Size::new(MIGRATION_SIZE.width * 2 + 8, MIGRATION_SIZE.height);
    let comparison = side_by_side(&legacy_pixels, &runtime_pixels, MIGRATION_SIZE, 8);
    let comparison_path = output.join("migration-first-batch-side-by-side-dark.png");
    write::png(&comparison_path, comparison_size, &comparison)?;

    let difference = pixel_difference(&legacy_pixels, &runtime_pixels);
    let difference_path = output.join("migration-first-batch-difference-dark.png");
    write::png(&difference_path, MIGRATION_SIZE, &difference)?;

    let report_path = output.join("migration-first-batch-layout.txt");
    write_migration_layout_report(&report_path, &legacy_layout, &runtime_layout)?;

    Ok(vec![
        legacy_path,
        runtime_path,
        comparison_path,
        difference_path,
        report_path,
    ])
}

fn migration_legacy_view(
    theme: ThemeMode,
) -> Element<'static, MigrationLayoutMessage, Theme, Renderer> {
    let tokens = theme.tokens();
    let title = LayoutProbe::new(
        container(text("Migration fixture").size(20))
            .width(Length::Fill)
            .height(Length::Fixed(28.0))
            .align_y(iced::alignment::Vertical::Center),
        |bounds| MigrationLayoutMessage {
            component: "text",
            bounds,
        },
    );
    let input = LayoutProbe::new(
        LegacyInput::new("Branch", "release/issue-7")
            .on_input(|_| MigrationLayoutMessage {
                component: "text-input:event",
                bounds: LayoutBounds::new(0.0, 0.0, 0.0, 0.0),
            })
            .view(tokens)
            .map(|message| message),
        |bounds| MigrationLayoutMessage {
            component: "text-input",
            bounds,
        },
    );
    let button = LayoutProbe::new(
        LegacyButton::label("Run build")
            .kind(ButtonKind::Primary)
            .width(Length::Fixed(140.0))
            .on_press(MigrationLayoutMessage {
                component: "button:event",
                bounds: LayoutBounds::new(0.0, 0.0, 0.0, 0.0),
            })
            .view(tokens),
        |bounds| MigrationLayoutMessage {
            component: "button",
            bounds,
        },
    );
    let checkbox = LayoutProbe::new(
        container(
            LegacyCheckbox::new(true, "Notifications")
                .on_toggle(|_| MigrationLayoutMessage {
                    component: "checkbox:event",
                    bounds: LayoutBounds::new(0.0, 0.0, 0.0, 0.0),
                })
                .view(tokens),
        )
        .width(Length::Fixed(200.0)),
        |bounds| MigrationLayoutMessage {
            component: "checkbox",
            bounds,
        },
    );
    let actions = row![button, checkbox]
        .spacing(12)
        .height(Length::Fixed(32.0));
    let content = column![title, input, actions]
        .spacing(12)
        .width(Length::Fill);
    container(content)
        .padding(24)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn migration_runtime_scene(
    theme: ThemeMode,
) -> Result<(UiScene, Vec<MigrationLayoutMessage>), Box<dyn std::error::Error>> {
    let mut context = AppContext::new();
    context.set_theme(theme)?;
    let document = DocumentId::new(2).expect("migration fixture document ID is non-zero");
    let title = context.create_component(
        document,
        RuntimeText::new("Migration fixture").style(NodeStyle {
            foreground: Some(SemanticColorRole::Text),
            layout: std::sync::Arc::new(LayoutStyle {
                font_size: Some(20.0),
                font_weight: Some(400),
                ..LayoutStyle::default()
            }),
            text_vertical_alignment: TextVerticalAlignment::Center,
            ..NodeStyle::default()
        }),
    )?;
    let input = context.create_component(
        document,
        RuntimeTextInput::new("release/issue-7").label("Branch"),
    )?;
    let button = context.create_component(
        document,
        RuntimeButton::new("Run build").kind(ButtonKind::Primary),
    )?;
    let checkbox =
        context.create_component(document, RuntimeCheckbox::new("Notifications", true))?;
    let layout = [
        (
            "text",
            title.stable_id(),
            LayoutBounds::new(24.0, 24.0, 472.0, 28.0),
        ),
        (
            "text-input",
            input.stable_id(),
            LayoutBounds::new(24.0, 64.0, 472.0, 32.0),
        ),
        (
            "button",
            button.stable_id(),
            LayoutBounds::new(24.0, 108.0, 140.0, 32.0),
        ),
        (
            "checkbox",
            checkbox.stable_id(),
            LayoutBounds::new(176.0, 108.0, 200.0, 32.0),
        ),
    ];
    let mut mutations = MutationQueue::new();
    for (_, id, bounds) in layout {
        mutations.write_layout(
            id,
            nana_ui::runtime::LayoutBox {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
            },
        );
    }
    context.commit_mutations(mutations)?;
    let work = context.take_system_work();
    context.resolve_styles(&work.style)?;
    let extracted = context.world().extract_nodes(&work.render_extraction);
    let mut scene = UiScene::new();
    scene.apply_delta(extracted, work.render_removals.iter().copied());
    let layout = layout
        .into_iter()
        .map(|(component, _, bounds)| MigrationLayoutMessage { component, bounds })
        .collect();
    Ok((scene, layout))
}

pub(super) fn side_by_side(left: &[u8], right: &[u8], size: Size<u32>, gap: u32) -> Vec<u8> {
    let output_width = size.width * 2 + gap;
    let mut output = vec![0; (output_width * size.height * 4) as usize];
    for y in 0..size.height as usize {
        let source_start = y * size.width as usize * 4;
        let source_end = source_start + size.width as usize * 4;
        let row_start = y * output_width as usize * 4;
        output[row_start..row_start + size.width as usize * 4]
            .copy_from_slice(&left[source_start..source_end]);
        let right_start = row_start + (size.width + gap) as usize * 4;
        output[right_start..right_start + size.width as usize * 4]
            .copy_from_slice(&right[source_start..source_end]);
    }
    output
}

pub(super) fn pixel_difference(left: &[u8], right: &[u8]) -> Vec<u8> {
    left.chunks_exact(4)
        .zip(right.chunks_exact(4))
        .flat_map(|(left, right)| {
            let red = left[0].abs_diff(right[0]);
            let green = left[1].abs_diff(right[1]);
            let blue = left[2].abs_diff(right[2]);
            [red, green, blue, 255]
        })
        .collect()
}

fn write_migration_layout_report(
    path: &Path,
    legacy: &[MigrationLayoutMessage],
    runtime: &[MigrationLayoutMessage],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut report = String::from("component\ticed\truntime\tstrict_equal\n");
    for runtime_entry in runtime {
        let legacy_bounds = legacy
            .iter()
            .find(|entry| entry.component == runtime_entry.component)
            .map(|entry| entry.bounds);
        report.push_str(&format!(
            "{}\t{:?}\t{:?}\t{}\n",
            runtime_entry.component,
            legacy_bounds,
            runtime_entry.bounds,
            legacy_bounds == Some(runtime_entry.bounds)
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, report)?;
    Ok(())
}

fn runtime_scene_snapshot(
    renderer: &mut Renderer,
    output: &Path,
    name: &str,
    theme: ThemeMode,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(900, 500);
    let scene = runtime_scene(theme)?;
    let view =
        nana_ui::IcedSceneView::new(&scene, Size::new(size.width as f32, size.height as f32))?;
    let view: Element<'_, (), Theme, Renderer> = view.into();
    let pixels = snapshot(
        renderer,
        view,
        &theme.iced_theme(),
        theme.colors().background,
        size,
    );
    let path = output.join(name);
    write::png(&path, size, &pixels)?;
    Ok(path)
}

fn runtime_scene(theme: ThemeMode) -> Result<UiScene, Box<dyn std::error::Error>> {
    let mut context = AppContext::new();
    context.set_theme(theme)?;
    let document = DocumentId::new(1).expect("snapshot document ID is non-zero");
    let title = context.create_component(
        document,
        RuntimeText::new("Build queue").style(NodeStyle {
            foreground: Some(SemanticColorRole::Text),
            layout: std::sync::Arc::new(LayoutStyle {
                font_size: Some(20.0),
                font_weight: Some(600),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        }),
    )?;
    let input = context.create_component(
        document,
        RuntimeTextInput::new("release/issue-7").label("Branch"),
    )?;
    let button = context.create_component(document, RuntimeButton::new("Run build"))?;
    let table = context.create_component(document, RuntimeTable::new().label("Recent builds"))?;
    let checkbox =
        context.create_component(document, RuntimeCheckbox::new("Notifications", true))?;
    let toggle = context.create_component(document, RuntimeSwitch::new("Auto build", true))?;
    let slider = context.create_component(
        document,
        RuntimeSlider::new(68.0, 0.0, 100.0)?.label("Volume"),
    )?;
    let tabs = context.create_component(document, RuntimeTabList::new().label("Output"))?;
    let preview = context.create_component(document, RuntimeTab::new("Preview").selected(true))?;
    let program = context.create_component(document, RuntimeTab::new("Program"))?;
    context.append_child(tabs, preview)?;
    context.append_child(tabs, program)?;
    let scroll_component = RuntimeScrollView::new(ScrollAxes::Vertical)
        .label("Activity")
        .style(NodeStyle {
            background: Some(SemanticColorRole::Surface),
            border: Some(SemanticColorRole::Border),
            layout: std::sync::Arc::new(LayoutStyle {
                border_width: Some(1.0),
                border_radius: Some(6.0),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        });
    let activity = context.create_component(document, scroll_component)?;
    context.scroll_to(activity, ScrollOffset { x: 0.0, y: 8.0 })?;
    let activity_lines = [
        context.create_component(document, RuntimeText::new("Queued #1043"))?,
        context.create_component(document, RuntimeText::new("Built #1042"))?,
        context.create_component(document, RuntimeText::new("Published artifacts"))?,
    ];
    for line in activity_lines {
        context.append_child(activity, line)?;
    }
    let card = context.create_component(document, RuntimeCard::new().label("Source inspector"))?;
    let card_title = context.create_component(document, RuntimeText::new("Source inspector"))?;
    let add_source = context.create_component(
        document,
        RuntimeIconButton::new(nana_ui::Icon::Add, "Add source"),
    )?;
    let notes = context.create_component(
        document,
        RuntimeTextArea::new("Camera follows Program.\nAudio monitoring enabled.")
            .label("Source notes"),
    )?;
    let source_list =
        context.create_component(document, RuntimeList::new().label("Scene sources"))?;
    let source_items = [
        context.create_component(document, RuntimeListItem::new("Camera").selected(true))?,
        context.create_component(document, RuntimeListItem::new("Live2D actor"))?,
        context.create_component(document, RuntimeListItem::new("Lower third").disabled(true))?,
    ];
    context.append_child(card, card_title)?;
    context.append_child(card, add_source)?;
    context.append_child(card, notes)?;
    context.append_child(card, source_list)?;
    for item in source_items {
        context.append_child(source_list, item)?;
    }

    let rows = [
        ["Build", "Status", "Duration"],
        ["#1042", "Succeeded", "1m 18s"],
        ["#1041", "Succeeded", "1m 21s"],
        ["#1040", "Failed", "42s"],
    ];
    let mut cells = Vec::new();
    for (row_index, values) in rows.into_iter().enumerate() {
        let row = context.create_component(document, RuntimeTableRow::new())?;
        context.append_child(table, row)?;
        for value in values {
            let style = NodeStyle {
                foreground: Some(if row_index == 0 {
                    SemanticColorRole::Muted
                } else {
                    SemanticColorRole::Text
                }),
                background: Some(if row_index == 0 {
                    SemanticColorRole::Subtle
                } else {
                    SemanticColorRole::Surface
                }),
                border: Some(SemanticColorRole::Border),
                layout: std::sync::Arc::new(LayoutStyle {
                    padding_left: Some(nana_ui_core::LengthSpec::Px(10.0)),
                    padding_right: Some(nana_ui_core::LengthSpec::Px(10.0)),
                    border_width: Some(1.0),
                    font_weight: (row_index == 0).then_some(600),
                    ..LayoutStyle::default()
                }),
                text_vertical_alignment: TextVerticalAlignment::Center,
                ..NodeStyle::default()
            };
            let cell = context.create_component(
                document,
                RuntimeTableCell::new(value)
                    .column_header(row_index == 0)
                    .style(style),
            )?;
            context.append_child(row, cell)?;
            cells.push(cell.stable_id());
        }
    }

    let mut layout = MutationQueue::new();
    for (id, bounds) in [
        (
            title.stable_id(),
            LayoutBox {
                x: 28.0,
                y: 24.0,
                width: 584.0,
                height: 28.0,
            },
        ),
        (
            input.stable_id(),
            LayoutBox {
                x: 28.0,
                y: 66.0,
                width: 390.0,
                height: 36.0,
            },
        ),
        (
            button.stable_id(),
            LayoutBox {
                x: 430.0,
                y: 66.0,
                width: 182.0,
                height: 36.0,
            },
        ),
        (
            table.stable_id(),
            LayoutBox {
                x: 28.0,
                y: 122.0,
                width: 584.0,
                height: 208.0,
            },
        ),
        (
            checkbox.stable_id(),
            LayoutBox {
                x: 28.0,
                y: 338.0,
                width: 170.0,
                height: 32.0,
            },
        ),
        (
            toggle.stable_id(),
            LayoutBox {
                x: 220.0,
                y: 338.0,
                width: 170.0,
                height: 32.0,
            },
        ),
        (
            slider.stable_id(),
            LayoutBox {
                x: 420.0,
                y: 338.0,
                width: 192.0,
                height: 32.0,
            },
        ),
        (
            tabs.stable_id(),
            LayoutBox {
                x: 28.0,
                y: 390.0,
                width: 584.0,
                height: 36.0,
            },
        ),
        (
            preview.stable_id(),
            LayoutBox {
                x: 28.0,
                y: 390.0,
                width: 116.0,
                height: 36.0,
            },
        ),
        (
            program.stable_id(),
            LayoutBox {
                x: 152.0,
                y: 390.0,
                width: 116.0,
                height: 36.0,
            },
        ),
        (
            activity.stable_id(),
            LayoutBox {
                x: 300.0,
                y: 390.0,
                width: 312.0,
                height: 76.0,
            },
        ),
        (
            activity_lines[0].stable_id(),
            LayoutBox {
                x: 312.0,
                y: 398.0,
                width: 288.0,
                height: 24.0,
            },
        ),
        (
            activity_lines[1].stable_id(),
            LayoutBox {
                x: 312.0,
                y: 426.0,
                width: 288.0,
                height: 24.0,
            },
        ),
        (
            activity_lines[2].stable_id(),
            LayoutBox {
                x: 312.0,
                y: 454.0,
                width: 288.0,
                height: 24.0,
            },
        ),
        (
            card.stable_id(),
            LayoutBox {
                x: 636.0,
                y: 24.0,
                width: 236.0,
                height: 442.0,
            },
        ),
        (
            card_title.stable_id(),
            LayoutBox {
                x: 652.0,
                y: 40.0,
                width: 160.0,
                height: 28.0,
            },
        ),
        (
            add_source.stable_id(),
            LayoutBox {
                x: 824.0,
                y: 36.0,
                width: 32.0,
                height: 32.0,
            },
        ),
        (
            notes.stable_id(),
            LayoutBox {
                x: 652.0,
                y: 84.0,
                width: 204.0,
                height: 112.0,
            },
        ),
        (
            source_list.stable_id(),
            LayoutBox {
                x: 652.0,
                y: 216.0,
                width: 204.0,
                height: 140.0,
            },
        ),
        (
            source_items[0].stable_id(),
            LayoutBox {
                x: 652.0,
                y: 216.0,
                width: 204.0,
                height: 36.0,
            },
        ),
        (
            source_items[1].stable_id(),
            LayoutBox {
                x: 652.0,
                y: 258.0,
                width: 204.0,
                height: 36.0,
            },
        ),
        (
            source_items[2].stable_id(),
            LayoutBox {
                x: 652.0,
                y: 300.0,
                width: 204.0,
                height: 36.0,
            },
        ),
    ] {
        layout.write_layout(id, bounds);
    }
    let column_widths = [180.0, 244.0, 160.0];
    for (index, id) in cells.into_iter().enumerate() {
        let row = index / column_widths.len();
        let column = index % column_widths.len();
        layout.write_layout(
            id,
            LayoutBox {
                x: 28.0 + column_widths[..column].iter().sum::<f32>(),
                y: 122.0 + row as f32 * 48.0,
                width: column_widths[column],
                height: 48.0,
            },
        );
    }
    context.commit_mutations(layout)?;
    let work = context.take_system_work();
    context.resolve_styles(&work.style)?;
    let mut scene = UiScene::new();
    scene.apply_delta(context.world().extract_document(document), []);
    Ok(scene)
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
        root: DockNode::item("navigation"),
        bounds: DockBounds::new(120.0, 120.0, size.width as f32, size.height as f32),
        monitor: None,
    });
    let mut controller = DockController::new(
        "editor",
        [
            DockItemSpec::new("editor", "Editor").closeable(false),
            DockItemSpec::new("navigation", "导航"),
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
            "navigation",
            container(
                column![
                    text("Section A").size(13).color(colors.text),
                    text("工作区导航").size(11).color(colors.muted),
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
            DockItemSpec::new("navigation", "导航"),
            DockItemSpec::new("console", "控制台"),
            DockItemSpec::new("output", "输出"),
        ],
        layout,
    )?;
    controller.set_chrome_style(DockChromeStyle::Card);
    controller.set_floating_window_title("NanaUI Gallery");
    let window_chrome = WindowChromeState::new(chrome);
    let colors = theme.colors();
    let contents = DockContents::new()
        .insert(
            "navigation",
            container(
                column![
                    text("Section A").size(13).color(colors.text),
                    text("工作区导航").size(11).color(colors.muted),
                ]
                .spacing(6),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .insert(
            "console",
            container(
                column![
                    text("Console").size(13).color(colors.text),
                    text("应用运行输出").size(11).color(colors.muted),
                ]
                .spacing(6),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .insert(
            "output",
            container(
                column![
                    text("Output").size(13).color(colors.text),
                    text("应用提供的输出内容").size(11).color(colors.muted),
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
            DockItemSpec::new("navigation", "导航"),
        ],
        DockLayout::new(DockNode::split(
            nana_ui::DockAxis::Horizontal,
            0.5,
            DockNode::item("navigation"),
            DockNode::item("editor"),
        )),
    )?;
    controller.update(DockAction::SurfaceGeometry {
        surface,
        bounds: DockBounds::new(0.0, 0.0, size.width as f32, size.height as f32),
    });
    controller.update(DockAction::DragStart {
        surface,
        id: DockId::from("navigation"),
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
        DockPreviewPhase::Settled => {
            std::thread::sleep(std::time::Duration::from_millis(100));
            controller.update(DockAction::Hover(false));
        }
        DockPreviewPhase::Retarget => {
            std::thread::sleep(std::time::Duration::from_millis(100));
            controller.update(DockAction::Hover(false));
            controller.update(DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(200.0, 50.0),
            });
            std::thread::sleep(std::time::Duration::from_millis(100));
            controller.update(DockAction::Hover(false));
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

fn gallery_snapshot_with_cursor(
    renderer: &mut Renderer,
    output: &Path,
    name: &str,
    state: &GalleryState,
    cursor: mouse::Cursor,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let pixels = snapshot_with_cursor(
        renderer,
        state.view(),
        &state.theme_mode().iced_theme(),
        state.theme_mode().colors().background,
        GALLERY_SIZE,
        cursor,
    );
    let path = output.join(name);
    write::png(&path, GALLERY_SIZE, &pixels)?;
    Ok(path)
}

fn prepare_dock_preview(state: &mut GalleryState) {
    let surface = DockSurfaceId(0);
    state.update(GalleryMessage::Dock(DockAction::DragStart {
        surface,
        id: DockId::from("gallery.assets"),
    }));
    state.update(GalleryMessage::Dock(DockAction::DragMove {
        surface,
        position: Point::new(350.0, 250.0),
    }));
    state.update(GalleryMessage::Dock(DockAction::DragMove {
        surface,
        position: Point::new(355.0, 250.0),
    }));
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
        .center(
            row![
                text("LiliaCode").size(12).color(colors.muted),
                text("›").size(12).color(colors.faint),
                text("恢复 Native 侧边栏交互与主界面布局")
                    .size(13)
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .ellipsis(iced::widget::text::Ellipsis::End),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .center_width(420.0)
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

fn snapshot_with_messages<Message>(
    renderer: &mut Renderer,
    view: Element<'_, Message, Theme, Renderer>,
    theme: &Theme,
    background: Color,
    size: Size<u32>,
) -> (Vec<u8>, Vec<Message>) {
    let viewport = Viewport::with_physical_size(size, renderer::Scale::default());
    let mut interface = UserInterface::build(
        view,
        viewport.logical_size(),
        user_interface::Cache::new(),
        renderer,
    );
    let window = window::Headless;
    let waker = shell::Waker::noop();
    let mut messages = shell::Bus::new();
    let _ = interface.update(
        &window,
        &waker,
        &[Event::Window(
            window::Event::RedrawRequested(Instant::now()),
        )],
        mouse::Cursor::Unavailable,
        renderer,
        &mut messages,
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
    (pixels, messages.drain().collect())
}

pub(super) fn snapshot_with_cursor<Message>(
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
        &mut shell::Bus::new(),
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
