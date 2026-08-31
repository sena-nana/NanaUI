use std::path::{Path, PathBuf};
use std::sync::Arc;

use component_gallery::{
    GalleryContextMenuEvent, GalleryMessage, GallerySection, GalleryState, SurfaceView,
};
use nana_ui::runtime::{
    AppShell, AppTitleBar, AppTitleBarControls, Button as RuntimeButton, Card as RuntimeCard,
    Checkbox as RuntimeCheckbox, Dock as RuntimeDock, DockAxis, DockDropZone, DockNode, DocumentId,
    IconButton as RuntimeIconButton, LayoutBox, LayoutViewport, List as RuntimeList,
    ListItem as RuntimeListItem, MutationQueue, NodeStyle, RangeField as RuntimeRangeField,
    RuntimeDocument, ScrollAxes, ScrollOffset, ScrollView as RuntimeScrollView,
    Switch as RuntimeSwitch, TabOption as RuntimeTabOption, Table as RuntimeTable,
    TableCell as RuntimeTableCell, TableRow as RuntimeTableRow, Tabs as RuntimeTabs,
    Text as RuntimeText, TextArea as RuntimeTextArea, TextInput as RuntimeTextInput,
    TextVerticalAlignment,
};
use nana_ui::{
    ButtonKind, CommandPaletteEvent, ControlSize, Icon, LogicalPoint, LogicalRect, NanaTextShaper,
    RuntimeInputAdapter, SettingsTabId, ThemeMode, ThemeModeExt, WindowChrome, WorkspaceAction,
};
use nana_ui_core::{LayoutStyle, LengthSpec, SemanticColorRole};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};

use crate::write::{self, Size};

#[path = "render/gpu.rs"]
mod gpu;
#[path = "render/migration_next.rs"]
mod migration_next;
#[path = "render/offscreen.rs"]
mod offscreen;

use offscreen::OffscreenSnapshots;

const GALLERY_SIZE: Size<u32> = Size::new(1280, 800);
const MIGRATION_SIZE: Size<u32> = Size::new(520, 220);

#[derive(Clone, Copy)]
enum DockPreviewPhase {
    Candidate,
    Settled,
    Retarget,
}

pub fn generate() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut snapshots = OffscreenSnapshots::new()?;
    let output = std::env::var_os("NANA_UI_SNAPSHOT_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?.join("target/ui-snapshots"));

    let mut paths = vec![
        runtime_scene_snapshot(
            &mut snapshots,
            &output,
            "runtime-scene-dark.png",
            ThemeMode::Dark,
        )?,
        runtime_scene_snapshot(
            &mut snapshots,
            &output,
            "runtime-scene-light.png",
            ThemeMode::Light,
        )?,
        titlebar_snapshot(
            &mut snapshots,
            &output,
            "titlebar-custom-dark.png",
            ThemeMode::Dark,
            WindowChrome::custom(),
            Some(LogicalPoint::new(880.0, 18.0)),
        )?,
        titlebar_snapshot(
            &mut snapshots,
            &output,
            "titlebar-custom-light.png",
            ThemeMode::Light,
            WindowChrome::custom(),
            None,
        )?,
        titlebar_snapshot(
            &mut snapshots,
            &output,
            "titlebar-native-leading-dark.png",
            ThemeMode::Dark,
            WindowChrome::native_leading(78.0),
            None,
        )?,
        dock_window_snapshot(
            &mut snapshots,
            &output,
            "dock-window-custom-dark.png",
            ThemeMode::Dark,
            WindowChrome::custom(),
            DockNode::item("navigation", None),
        )?,
        dock_window_snapshot(
            &mut snapshots,
            &output,
            "dock-window-native-leading-light.png",
            ThemeMode::Light,
            WindowChrome::native_leading(78.0),
            DockNode::item("navigation", None),
        )?,
    ];
    paths.extend(component_migration_snapshots(
        &mut snapshots,
        &output,
        ThemeMode::Dark,
    )?);
    for theme in [ThemeMode::Dark, ThemeMode::Light] {
        paths.extend(migration_next::generate_registered(
            &mut snapshots,
            &output,
            theme,
        )?);
    }

    for (suffix, theme) in [("dark", ThemeMode::Dark), ("light", ThemeMode::Light)] {
        paths.push(dock_window_snapshot(
            &mut snapshots,
            &output,
            &format!("dock-window-merged-tabs-{suffix}.png"),
            theme,
            WindowChrome::custom(),
            DockNode::tabs(
                ["navigation", "console", "output"],
                "console",
                [("navigation", None), ("console", None), ("output", None)],
            ),
        )?);
        paths.push(dock_window_snapshot(
            &mut snapshots,
            &output,
            &format!("dock-window-merged-split-{suffix}.png"),
            theme,
            WindowChrome::custom(),
            DockNode::split(
                DockAxis::Horizontal,
                0.5,
                DockNode::tabs(
                    ["navigation", "console"],
                    "navigation",
                    [("navigation", None), ("console", None)],
                ),
                DockNode::item("output", None),
            ),
        )?);
        paths.push(dock_drag_window_snapshot(
            &mut snapshots,
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
                &mut snapshots,
                &output,
                &format!("dock-preview-{name}-{suffix}.png"),
                theme,
                zone,
                DockPreviewPhase::Settled,
            )?);
        }
        paths.push(dock_preview_snapshot(
            &mut snapshots,
            &output,
            &format!("dock-preview-retarget-tab-{suffix}.png"),
            theme,
            DockDropZone::Left,
            DockPreviewPhase::Retarget,
        )?);
        paths.push(dock_preview_snapshot(
            &mut snapshots,
            &output,
            &format!("dock-hover-left-{suffix}.png"),
            theme,
            DockDropZone::Left,
            DockPreviewPhase::Candidate,
        )?);
        paths.push(dock_preview_snapshot(
            &mut snapshots,
            &output,
            &format!("dock-preview-outside-{suffix}.png"),
            theme,
            DockDropZone::Left,
            DockPreviewPhase::Candidate,
        )?);
    }

    let mut controls = GalleryState::new();
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-controls-dark.png",
        &mut controls,
    )?);

    let mut controls_light = GalleryState::new();
    controls_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-controls-light.png",
        &mut controls_light,
    )?);
    paths.push(gallery_snapshot_with_cursor(
        &mut snapshots,
        &output,
        "gallery-sidebar-tools-dark.png",
        &mut controls,
        LogicalPoint::new(180.0, 60.0),
    )?);
    paths.push(gallery_snapshot_with_cursor(
        &mut snapshots,
        &output,
        "gallery-sidebar-tools-light.png",
        &mut controls_light,
        LogicalPoint::new(180.0, 60.0),
    )?);

    let mut loading = GalleryState::new();
    loading.update(GalleryMessage::ToggleLoading);
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-loading-dark.png",
        &mut loading,
    )?);

    let mut surfaces = GalleryState::new();
    surfaces.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-surfaces-dark.png",
        &mut surfaces,
    )?);

    let mut surfaces_light = GalleryState::new();
    surfaces_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    surfaces_light.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-surfaces-light.png",
        &mut surfaces_light,
    )?);

    surfaces.update(GalleryMessage::PaneChrome(
        nana_ui::PaneChromeActionKind::SplitHorizontal,
    ));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-surfaces-split-dark.png",
        &mut surfaces,
    )?);

    surfaces_light.update(GalleryMessage::PaneChrome(
        nana_ui::PaneChromeActionKind::SplitHorizontal,
    ));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-surfaces-split-light.png",
        &mut surfaces_light,
    )?);

    let mut cards = GalleryState::new();
    cards.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    cards.update(GalleryMessage::SelectSurfaceView(SurfaceView::Cards));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-cards-dark.png",
        &mut cards,
    )?);

    surfaces_light.update(GalleryMessage::SelectSurfaceView(SurfaceView::Cards));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-cards-light.png",
        &mut surfaces_light,
    )?);

    let mut feedback = GalleryState::new();
    feedback.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-feedback-dark.png",
        &mut feedback,
    )?);

    let mut rich_text = GalleryState::new();
    rich_text.update(GalleryMessage::SelectSection(GallerySection::RichText));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-rich-text-dark.png",
        &mut rich_text,
    )?);
    rich_text.update(GalleryMessage::SetTheme(ThemeMode::Light));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-rich-text-light.png",
        &mut rich_text,
    )?);

    let mut popover = GalleryState::new();
    popover.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    popover.update(GalleryMessage::TogglePopover);
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-popover-dark.png",
        &mut popover,
    )?);

    let mut context_menu = GalleryState::new();
    context_menu.update(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
        width: GALLERY_SIZE.width as f32,
        height: GALLERY_SIZE.height as f32,
    }));
    context_menu.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    context_menu.update(GalleryMessage::ToggleContextMenu);
    context_menu.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::OpenSubmenu(vec![0]),
    ));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-context-menu-dark.png",
        &mut context_menu,
    )?);
    context_menu.update(GalleryMessage::SetTheme(ThemeMode::Light));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-context-menu-light.png",
        &mut context_menu,
    )?);

    let mut context_menu_search = GalleryState::new();
    context_menu_search.update(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
        width: GALLERY_SIZE.width as f32,
        height: GALLERY_SIZE.height as f32,
    }));
    context_menu_search.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    context_menu_search.update(GalleryMessage::ToggleContextMenu);
    context_menu_search.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::Search("重命名".to_owned()),
    ));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-context-menu-search-dark.png",
        &mut context_menu_search,
    )?);

    let mut context_menu_search_light = GalleryState::new();
    context_menu_search_light.update(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
        width: GALLERY_SIZE.width as f32,
        height: GALLERY_SIZE.height as f32,
    }));
    context_menu_search_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    context_menu_search_light.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    context_menu_search_light.update(GalleryMessage::ToggleContextMenu);
    context_menu_search_light.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::Search("copy".to_owned()),
    ));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-context-menu-search-light.png",
        &mut context_menu_search_light,
    )?);

    let mut command_palette = GalleryState::new();
    command_palette.update(GalleryMessage::ToggleCommandPalette);
    command_palette.update(GalleryMessage::CommandPalette(CommandPaletteEvent::Search(
        "工作区".to_owned(),
    )));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-command-palette-dark.png",
        &mut command_palette,
    )?);

    let mut command_palette_light = GalleryState::new();
    command_palette_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    command_palette_light.update(GalleryMessage::ToggleCommandPalette);
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-command-palette-light.png",
        &mut command_palette_light,
    )?);

    let mut dialog = GalleryState::new();
    dialog.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    dialog.update(GalleryMessage::ToggleDialog);
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-dialog-dark.png",
        &mut dialog,
    )?);

    let mut image_viewer = GalleryState::new();
    image_viewer.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    image_viewer.update(GalleryMessage::ToggleImageViewer);
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-image-viewer-dark.png",
        &mut image_viewer,
    )?);

    let mut workspace = GalleryState::new();
    workspace.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-workspace-dark.png",
        &mut workspace,
    )?);

    let mut workspace_dock_preview = GalleryState::new();
    workspace_dock_preview.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-workspace-dock-preview-dark.png",
        &mut workspace_dock_preview,
    )?);
    let mut workspace_dock_preview_light = GalleryState::new();
    workspace_dock_preview_light.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    workspace_dock_preview_light.update(GalleryMessage::SetTheme(ThemeMode::Light));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-workspace-dock-preview-light.png",
        &mut workspace_dock_preview_light,
    )?);

    let mut sidebar_collapsed = GalleryState::new();
    sidebar_collapsed.update(GalleryMessage::Workspace(
        WorkspaceAction::SetRegionCollapsed(nana_ui::RegionId::Resources, true),
    ));
    sidebar_collapsed.update(GalleryMessage::Workspace(WorkspaceAction::AnimationFrame(
        std::time::Duration::from_millis(300),
    )));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-sidebar-collapsed-dark.png",
        &mut sidebar_collapsed,
    )?);

    let mut settings = GalleryState::new();
    settings.update(GalleryMessage::OpenSettings);
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-settings-appearance-dark.png",
        &mut settings,
    )?);

    settings.update(GalleryMessage::SetTheme(ThemeMode::Light));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-settings-appearance-light.png",
        &mut settings,
    )?);

    settings.update(GalleryMessage::SetTheme(ThemeMode::Dark));
    settings.update(GalleryMessage::SelectSettingsTab(SettingsTabId::from(
        "workspace",
    )));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-settings-workspace-dark.png",
        &mut settings,
    )?);

    settings.update(GalleryMessage::SelectSettingsTab(SettingsTabId::from(
        "about",
    )));
    paths.push(gallery_snapshot(
        &mut snapshots,
        &output,
        "gallery-settings-about-dark.png",
        &mut settings,
    )?);

    Ok(paths)
}

#[derive(Debug, Clone, Copy)]
struct MigrationLayoutMessage {
    component: &'static str,
    bounds: LogicalRect,
}

fn component_migration_snapshots(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
    theme: ThemeMode,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let (runtime_document, runtime_layout) = migration_runtime_document(theme)?;
    let runtime_pixels = snapshots.paint(
        runtime_document.scene(),
        MIGRATION_SIZE,
        clear_color(theme),
        None,
        None,
    )?;
    let runtime_path = output.join("migration-first-batch-runtime-dark.png");
    write::png(&runtime_path, MIGRATION_SIZE, &runtime_pixels)?;

    let legacy_path = output.join("migration-first-batch-reference-dark.png");
    let legacy_pixels = archived_or_runtime(&legacy_path, MIGRATION_SIZE, &runtime_pixels)?;

    let comparison_size = Size::new(MIGRATION_SIZE.width * 2 + 8, MIGRATION_SIZE.height);
    let comparison = side_by_side(&legacy_pixels, &runtime_pixels, MIGRATION_SIZE, 8);
    let comparison_path = output.join("migration-first-batch-side-by-side-dark.png");
    write::png(&comparison_path, comparison_size, &comparison)?;

    let difference = pixel_difference(&legacy_pixels, &runtime_pixels);
    let difference_path = output.join("migration-first-batch-difference-dark.png");
    write::png(&difference_path, MIGRATION_SIZE, &difference)?;

    let report_path = output.join("migration-first-batch-layout.txt");
    write_migration_layout_report(&report_path, &runtime_layout)?;

    Ok(vec![
        legacy_path,
        runtime_path,
        comparison_path,
        difference_path,
        report_path,
    ])
}

fn migration_runtime_document(
    theme: ThemeMode,
) -> Result<(RuntimeDocument, Vec<MigrationLayoutMessage>), Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(2).expect("migration fixture document ID is non-zero");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_theme(theme)?;
    let mut root_style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut root_style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.padding_left = Some(LengthSpec::Px(24.0));
        layout.padding_right = Some(LengthSpec::Px(24.0));
        layout.padding_top = Some(LengthSpec::Px(24.0));
        layout.padding_bottom = Some(LengthSpec::Px(24.0));
        layout.gap = Some(LengthSpec::Px(12.0));
    }
    let (title, input, button, checkbox) = document.context_mut().build(document_id, |ui| {
        let root = ui.child("root", RuntimeList::new().style(root_style));
        ui.nest(root, |ui| {
            let title = ui.child(
                "title",
                RuntimeText::new("Migration fixture").style(NodeStyle {
                    foreground: Some(SemanticColorRole::Text),
                    layout: Arc::new(LayoutStyle {
                        font_size: Some(20.0),
                        font_weight: Some(400),
                        width: Some(LengthSpec::Fill),
                        height: Some(LengthSpec::Px(28.0)),
                        ..LayoutStyle::default()
                    }),
                    text_vertical_alignment: TextVerticalAlignment::Center,
                    ..NodeStyle::default()
                }),
            );
            let input = ui.child(
                "input",
                RuntimeTextInput::new("release/issue-7").label("Branch"),
            );
            let button = ui.child(
                "button",
                RuntimeButton::new("Run build").kind(ButtonKind::Primary),
            );
            let checkbox = ui.child("checkbox", RuntimeCheckbox::new("Notifications", true));
            (title, input, button, checkbox)
        })
    })?;
    document.flush(
        LayoutViewport::new(MIGRATION_SIZE.width as f32, MIGRATION_SIZE.height as f32),
        &mut NanaTextShaper::default(),
    )?;
    let layout = ["text", "text-input", "button", "checkbox"]
        .into_iter()
        .zip([
            title.stable_id(),
            input.stable_id(),
            button.stable_id(),
            checkbox.stable_id(),
        ])
        .filter_map(|(component, id)| {
            document
                .context()
                .world()
                .layout_box(id)
                .map(|bounds| MigrationLayoutMessage {
                    component,
                    bounds: LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
                })
        })
        .collect();
    Ok((document, layout))
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
    left.as_chunks::<4>()
        .0
        .iter()
        .zip(right.as_chunks::<4>().0)
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
    runtime: &[MigrationLayoutMessage],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut report = String::from("component\tarchived\truntime\tstrict_equal\n");
    for entry in runtime {
        report.push_str(&format!(
            "{}\tarchived-png\t{:?}\tfalse\n",
            entry.component, entry.bounds
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, report)?;
    Ok(())
}

fn runtime_scene_snapshot(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
    name: &str,
    theme: ThemeMode,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(900, 500);
    let document = runtime_scene_document(theme)?;
    offscreen::write_scene(
        snapshots,
        output,
        name,
        document.scene(),
        size,
        clear_color(theme),
        None,
        None,
    )
}

fn runtime_scene_document(theme: ThemeMode) -> Result<RuntimeDocument, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(1).expect("snapshot document ID is non-zero");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_theme(theme)?;
    let slider_component = RuntimeRangeField::new(68.0, 0.0, 100.0, 1.0)?.label("Volume");
    let (
        title,
        input,
        button,
        table,
        checkbox,
        toggle,
        slider,
        tabs,
        activity,
        activity_lines,
        card,
        add_source,
        notes,
        source_list,
        source_items,
        cells,
    ) = document.context_mut().build(document_id, |ui| {
        let title = ui.child(
            "title",
            RuntimeText::new("Build queue").style(NodeStyle {
                foreground: Some(SemanticColorRole::Text),
                layout: Arc::new(LayoutStyle {
                    font_size: Some(20.0),
                    font_weight: Some(600),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            }),
        );
        let input = ui.child(
            "input",
            RuntimeTextInput::new("release/issue-7").label("Branch"),
        );
        let button = ui.child("button", RuntimeButton::new("Run build"));
        let table = ui.child("table", RuntimeTable::new().label("Recent builds"));
        let checkbox = ui.child("checkbox", RuntimeCheckbox::new("Notifications", true));
        let toggle = ui.child("toggle", RuntimeSwitch::new("Auto build", true));
        let slider = ui.child("slider", slider_component);
        let tabs = ui.child(
            "tabs",
            RuntimeTabs::new("preview").label("Output").options([
                RuntimeTabOption::new("preview", "Preview"),
                RuntimeTabOption::new("program", "Program"),
            ]),
        );
        let activity = ui.child(
            "activity",
            RuntimeScrollView::new(ScrollAxes::Vertical)
                .label("Activity")
                .style(NodeStyle {
                    background: Some(SemanticColorRole::Surface),
                    border: Some(SemanticColorRole::Border),
                    layout: Arc::new(LayoutStyle {
                        border_width: Some(1.0),
                        border_radius: Some(6.0),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                }),
        );
        let activity_lines = ui.nest(activity, |ui| {
            [
                ui.child("queued", RuntimeText::new("Queued #1043")),
                ui.child("built", RuntimeText::new("Built #1042")),
                ui.child("published", RuntimeText::new("Published artifacts")),
            ]
        });
        let card = ui.child("card", RuntimeCard::new().label("Source inspector"));
        let (add_source, notes, source_list, source_items) = ui.nest(card, |ui| {
            let add_source = ui.child(
                "add-source",
                RuntimeIconButton::new(nana_ui::Icon::Add, "Add source"),
            );
            let notes = ui.child(
                "notes",
                RuntimeTextArea::new("Camera follows Program.\nAudio monitoring enabled.")
                    .label("Source notes"),
            );
            let source_list = ui.child("sources", RuntimeList::new().label("Scene sources"));
            let source_items = ui.nest(source_list, |ui| {
                [
                    ui.child("camera", RuntimeListItem::new("Camera").selected(true)),
                    ui.child("actor", RuntimeListItem::new("Live2D actor")),
                    ui.child(
                        "lower-third",
                        RuntimeListItem::new("Lower third").disabled(true),
                    ),
                ]
            });
            (add_source, notes, source_list, source_items)
        });
        let rows = [
            ["Build", "Status", "Duration"],
            ["#1042", "Succeeded", "1m 18s"],
            ["#1041", "Succeeded", "1m 21s"],
            ["#1040", "Failed", "42s"],
        ];
        let mut cells = Vec::new();
        ui.nest(table, |ui| {
            for (row_index, values) in rows.into_iter().enumerate() {
                let row = ui.child(format!("row-{row_index}"), RuntimeTableRow::new());
                ui.nest(row, |ui| {
                    for (column, value) in values.into_iter().enumerate() {
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
                            layout: Arc::new(LayoutStyle {
                                padding_left: Some(LengthSpec::Px(10.0)),
                                padding_right: Some(LengthSpec::Px(10.0)),
                                border_width: Some(1.0),
                                font_weight: (row_index == 0).then_some(600),
                                ..LayoutStyle::default()
                            }),
                            text_vertical_alignment: TextVerticalAlignment::Center,
                            ..NodeStyle::default()
                        };
                        let cell = ui.child(
                            format!("cell-{column}"),
                            RuntimeTableCell::new(value)
                                .column_header(row_index == 0)
                                .style(style),
                        );
                        cells.push(cell.stable_id());
                    }
                });
            }
        });
        (
            title,
            input,
            button,
            table,
            checkbox,
            toggle,
            slider,
            tabs,
            activity,
            activity_lines,
            card,
            add_source,
            notes,
            source_list,
            source_items,
            cells,
        )
    })?;
    document
        .context_mut()
        .scroll_to(activity, ScrollOffset { x: 0.0, y: 8.0 })?;
    let option_ids = document.context().read(tabs, |tabs| {
        tabs.option_nodes()
            .iter()
            .map(|(_, id)| *id)
            .collect::<Vec<_>>()
    })?;
    let preview = option_ids[0];
    let program = option_ids[1];

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
            preview,
            LayoutBox {
                x: 28.0,
                y: 390.0,
                width: 116.0,
                height: 36.0,
            },
        ),
        (
            program,
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
    document.context_mut().commit_mutations(layout)?;
    document.flush_with(|context, work| {
        context.shape_text(&work.text, &mut NanaTextShaper::default())?;
        Ok(())
    })?;
    Ok(document)
}

fn titlebar_snapshot(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
    name: &str,
    theme: ThemeMode,
    chrome: WindowChrome,
    hover: Option<LogicalPoint>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(900, 120);
    let mut document = titlebar_document(theme, chrome)?;
    if let Some(point) = hover {
        dispatch_pointer(&mut document, size, PointerPhase::Move, point)?;
    }
    offscreen::write_scene(
        snapshots,
        output,
        name,
        document.scene(),
        size,
        clear_color(theme),
        None,
        None,
    )
}

fn titlebar_document(
    theme: ThemeMode,
    chrome: WindowChrome,
) -> Result<RuntimeDocument, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(3).expect("titlebar document");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_theme(theme)?;
    let title = document.context_mut().build(document_id, |ui| {
        let leading = ui.leaf(labeled_text("NANA", SemanticColorRole::Accent, 12.0, 600));
        let center = ui.leaf(labeled_text(
            "LiliaCode › 恢复 Native 侧边栏交互与主界面布局",
            SemanticColorRole::Text,
            13.0,
            400,
        ));
        let trailing = ui.leaf(labeled_text("Gallery", SemanticColorRole::Muted, 11.0, 400));
        let minimize = ui.leaf(window_control(Icon::Minimize, "Minimize"));
        let maximize = ui.leaf(window_control(Icon::Maximize, "Maximize"));
        let close = ui.leaf(window_control(Icon::Close, "Close"));
        let controls = ui.leaf(
            AppTitleBarControls::new(false)
                .minimize(minimize.stable_id())
                .maximize(maximize.stable_id())
                .close(close.stable_id()),
        );
        ui.nest(controls, |ui| {
            ui.adopt(minimize);
            ui.adopt(maximize);
            ui.adopt(close);
        });
        let title = ui.child(
            "title",
            AppTitleBar::new("NanaUI")
                .leading(leading.stable_id())
                .center(center.stable_id())
                .trailing(trailing.stable_id())
                .controls(controls.stable_id())
                .center_width(420.0)
                .leading_inset(chrome.leading_inset)
                .trailing_inset(chrome.trailing_inset)
                .show_window_controls(chrome.uses_custom_controls()),
        );
        ui.nest(title, |ui| {
            ui.adopt(leading);
            ui.adopt(center);
            ui.adopt(trailing);
            ui.adopt(controls);
        });
        title
    })?;
    document.context_mut().assemble_app_title_bar(title)?;
    document.flush(
        LayoutViewport::new(900.0, 120.0),
        &mut NanaTextShaper::default(),
    )?;
    Ok(document)
}

fn dock_window_snapshot(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
    name: &str,
    theme: ThemeMode,
    chrome: WindowChrome,
    root: DockNode,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = if matches!(
        name,
        n if n.contains("merged")
    ) {
        Size::new(520, 360)
    } else {
        Size::new(420, 320)
    };
    let document = dock_window_document(theme, chrome, root, size)?;
    offscreen::write_scene(
        snapshots,
        output,
        name,
        document.scene(),
        size,
        clear_color(theme),
        None,
        None,
    )
}

fn dock_window_document(
    theme: ThemeMode,
    chrome: WindowChrome,
    root: DockNode,
    size: Size<u32>,
) -> Result<RuntimeDocument, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(4).expect("dock window document");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_theme(theme)?;
    // Tab chrome only: pane text overlaps titles at this fixture size.
    let dock = RuntimeDock::new(root)
        .title("navigation", "导航")
        .title("console", "控制台")
        .title("output", "输出")
        .title("editor", "Editor");
    let (shell, dock) = document.context_mut().build(document_id, |ui| {
        let dock = ui.leaf(dock);
        let title = ui.leaf(
            AppTitleBar::new("NanaUI Gallery")
                .leading_inset(chrome.leading_inset)
                .trailing_inset(chrome.trailing_inset)
                .show_window_controls(chrome.uses_custom_controls()),
        );
        let shell = ui.child(
            "shell",
            AppShell::new()
                .title_bar(title.stable_id())
                .body(dock.stable_id()),
        );
        ui.nest(shell, |ui| {
            ui.adopt(title);
            ui.adopt(dock);
        });
        (shell, dock)
    })?;
    document.context_mut().assemble_dock(dock)?;
    document.context_mut().assemble_app_shell(shell)?;
    document.flush(
        LayoutViewport::new(size.width as f32, size.height as f32),
        &mut NanaTextShaper::default(),
    )?;
    Ok(document)
}

fn dock_drag_window_snapshot(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
    name: &str,
    theme: ThemeMode,
    chrome: WindowChrome,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(420, 240);
    let document = dock_window_document(theme, chrome, DockNode::item("navigation", None), size)?;
    offscreen::write_scene(
        snapshots,
        output,
        name,
        document.scene(),
        size,
        clear_color(theme),
        None,
        None,
    )
}

fn dock_preview_snapshot(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
    name: &str,
    theme: ThemeMode,
    zone: DockDropZone,
    phase: DockPreviewPhase,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let size = Size::new(420, 240);
    let document = dock_preview_document(theme, zone, phase, name.contains("outside"), size)?;
    offscreen::write_scene(
        snapshots,
        output,
        name,
        document.scene(),
        size,
        clear_color(theme),
        None,
        None,
    )
}

fn dock_preview_document(
    theme: ThemeMode,
    zone: DockDropZone,
    phase: DockPreviewPhase,
    outside: bool,
    size: Size<u32>,
) -> Result<RuntimeDocument, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(5).expect("dock preview document");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_theme(theme)?;
    let drop = if outside {
        None
    } else {
        let target = match (zone, phase) {
            (_, DockPreviewPhase::Retarget) => "editor",
            (DockDropZone::Left | DockDropZone::Top, _) => "source",
            (DockDropZone::Right | DockDropZone::Bottom | DockDropZone::Tab, _) => "editor",
        };
        let zone = if matches!(phase, DockPreviewPhase::Retarget) {
            DockDropZone::Tab
        } else {
            zone
        };
        Some((target, zone))
    };
    // Tab chrome only: pane text overlaps titles at this fixture size.
    let mut spec = RuntimeDock::new(DockNode::split(
        DockAxis::Horizontal,
        0.5,
        DockNode::item("source", None),
        DockNode::split(
            DockAxis::Vertical,
            0.5,
            DockNode::item("panel", None),
            DockNode::item("editor", None),
        ),
    ))
    .title("source", "Source")
    .title("panel", "Panel")
    .title("editor", "Editor");
    if let Some((target, zone)) = drop {
        spec = spec.drop_target(target, zone);
    }
    let dock = document.context_mut().create_component(document_id, spec)?;
    document.context_mut().assemble_dock(dock)?;
    document.flush(
        LayoutViewport::new(size.width as f32, size.height as f32),
        &mut NanaTextShaper::default(),
    )?;
    Ok(document)
}

fn gallery_snapshot(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
    name: &str,
    state: &mut GalleryState,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    state.flush_snapshot_scene();
    let pixels = paint_gallery(snapshots, state, GALLERY_SIZE)?;
    let path = output.join(name);
    write::png(&path, GALLERY_SIZE, &pixels)?;
    Ok(path)
}

fn gallery_snapshot_with_cursor(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
    name: &str,
    state: &mut GalleryState,
    cursor: LogicalPoint,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    state.flush_snapshot_scene();
    state.snapshot_hover(cursor.x, cursor.y);
    gallery_snapshot(snapshots, output, name, state)
}

fn paint_gallery(
    snapshots: &mut OffscreenSnapshots,
    state: &GalleryState,
    size: Size<u32>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let clear = clear_color(state.theme_mode());
    match state.active_scene() {
        Some(scene) => snapshots.paint(scene, size, clear, None, None),
        None => snapshots.paint_layers(&[], size, clear, None, None),
    }
}

fn archived_or_runtime(
    path: &Path,
    size: Size<u32>,
    runtime: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if let Some((png_size, pixels)) = write::read_png(path)
        && png_size == size
        && pixels.len() == runtime.len()
    {
        return Ok(pixels);
    }
    write::png(path, size, runtime)?;
    Ok(runtime.to_vec())
}

fn clear_color(theme: ThemeMode) -> [f32; 4] {
    let color = theme.colors().background;
    [color.r, color.g, color.b, 1.0]
}

fn labeled_text(
    value: impl Into<String>,
    color: SemanticColorRole,
    size: f32,
    weight: u16,
) -> RuntimeText {
    let mut style = NodeStyle {
        foreground: Some(color),
        ..NodeStyle::default()
    };
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(size);
    layout.font_weight = Some(weight);
    RuntimeText::new(value).style(style)
}

fn window_control(icon: Icon, label: &'static str) -> RuntimeIconButton {
    RuntimeIconButton::new(icon, label)
        .size(ControlSize::Small)
        .kind(ButtonKind::Text)
}

fn dispatch_pointer(
    document: &mut RuntimeDocument,
    size: Size<u32>,
    phase: PointerPhase,
    point: LogicalPoint,
) -> Result<(), Box<dyn std::error::Error>> {
    let document_id = document.document();
    RuntimeInputAdapter::default().dispatch(
        document.context_mut(),
        document_id,
        &InputEvent::Pointer {
            phase,
            pointer_id: 1,
            pointer_type: PointerType::Mouse,
            x: point.x,
            y: point.y,
            screen_x: point.x,
            screen_y: point.y,
            button: 0,
            buttons: 0,
            pressure: 0.5,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        },
    )?;
    document.flush(
        LayoutViewport::new(size.width as f32, size.height as f32),
        &mut NanaTextShaper::default(),
    )?;
    Ok(())
}
