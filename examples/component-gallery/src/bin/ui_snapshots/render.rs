use std::path::Path;
use std::sync::Arc;

use component_gallery::{
    GalleryContextMenuEvent, GalleryMessage, GallerySection, GalleryState, SurfaceView,
};
use nana_ui::runtime::view::{AnyView, IntoView, entity_ref, widget, with_refs};
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
    SettingsTabId, ThemeAppearance, WindowChrome, WorkspaceAction,
};
use nana_ui_core::{LayoutStyle, LengthSpec, SemanticColorRole, type_scale};
use nana_ui_platform::{InputPayload, PointerInput, PointerPhase};

use crate::baseline::{Recorder, Report};
use crate::write::Size;

#[path = "render/gpu.rs"]
mod gpu;
#[path = "render/migration_next.rs"]
mod migration_next;
#[path = "render/motion.rs"]
mod motion;
#[path = "render/offscreen.rs"]
mod offscreen;

use offscreen::OffscreenSnapshots;

const GALLERY_SIZE: Size<u32> = Size::new(1280, 800);
const MIGRATION_SIZE: Size<u32> = Size::new(520, 220);

/// Issue #101 §3: the state matrix as resolved style, with no GPU.
///
/// Separate from [`generate`] rather than a flag inside it, because the two
/// answer different questions and only one of them needs an adapter. Keeping
/// them apart is what lets the semantic baseline be verified anywhere.
pub fn generate_semantic(mut recorder: Recorder) -> Result<Report, Box<dyn std::error::Error>> {
    for theme in [ThemeAppearance::Dark, ThemeAppearance::Light] {
        migration_next::generate_semantic(&mut recorder, theme)?;
    }
    recorder.finish()
}

pub fn generate(mut recorder: Recorder) -> Result<Report, Box<dyn std::error::Error>> {
    let mut snapshots = OffscreenSnapshots::new()?;
    // Painter state is shared across the suite, so the render order is part of
    // what the baseline records: motion frames stay first.
    motion::generate(&mut snapshots, &mut recorder)?;
    runtime_scene_snapshot(
        &mut snapshots,
        &mut recorder,
        "runtime-scene-dark.png",
        ThemeAppearance::Dark,
    )?;
    runtime_scene_snapshot(
        &mut snapshots,
        &mut recorder,
        "runtime-scene-light.png",
        ThemeAppearance::Light,
    )?;
    titlebar_snapshot(
        &mut snapshots,
        &mut recorder,
        "titlebar-custom-dark.png",
        ThemeAppearance::Dark,
        WindowChrome::custom(),
        Some(LogicalPoint::new(880.0, 18.0)),
    )?;
    titlebar_snapshot(
        &mut snapshots,
        &mut recorder,
        "titlebar-custom-light.png",
        ThemeAppearance::Light,
        WindowChrome::custom(),
        None,
    )?;
    titlebar_snapshot(
        &mut snapshots,
        &mut recorder,
        "titlebar-native-leading-dark.png",
        ThemeAppearance::Dark,
        WindowChrome::native_leading(78.0),
        None,
    )?;
    dock_window_snapshot(
        &mut snapshots,
        &mut recorder,
        "dock-window-custom-dark.png",
        ThemeAppearance::Dark,
        WindowChrome::custom(),
        DockNode::item("navigation", None),
    )?;
    dock_window_snapshot(
        &mut snapshots,
        &mut recorder,
        "dock-window-native-leading-light.png",
        ThemeAppearance::Light,
        WindowChrome::native_leading(78.0),
        DockNode::item("navigation", None),
    )?;
    component_migration_snapshots(&mut snapshots, &mut recorder, ThemeAppearance::Dark)?;
    for theme in [ThemeAppearance::Dark, ThemeAppearance::Light] {
        migration_next::generate_registered(&mut snapshots, &mut recorder, theme)?;
    }

    for (suffix, theme) in [
        ("dark", ThemeAppearance::Dark),
        ("light", ThemeAppearance::Light),
    ] {
        dock_window_snapshot(
            &mut snapshots,
            &mut recorder,
            &format!("dock-window-merged-tabs-{suffix}.png"),
            theme,
            WindowChrome::custom(),
            DockNode::tabs(
                ["navigation", "console", "output"],
                "console",
                [("navigation", None), ("console", None), ("output", None)],
            ),
        )?;
        dock_window_snapshot(
            &mut snapshots,
            &mut recorder,
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
        )?;
        dock_drag_window_snapshot(
            &mut snapshots,
            &mut recorder,
            &format!("dock-drag-window-{suffix}.png"),
            theme,
            WindowChrome::custom(),
        )?;
        for (name, zone) in [
            ("left", DockDropZone::Left),
            ("right", DockDropZone::Right),
            ("top", DockDropZone::Top),
            ("bottom", DockDropZone::Bottom),
            ("tab", DockDropZone::Tab),
        ] {
            dock_preview_snapshot(
                &mut snapshots,
                &mut recorder,
                &format!("dock-preview-{name}-{suffix}.png"),
                theme,
                zone,
                false,
            )?;
        }
        dock_preview_snapshot(
            &mut snapshots,
            &mut recorder,
            &format!("dock-preview-outside-{suffix}.png"),
            theme,
            DockDropZone::Left,
            true,
        )?;
    }

    let mut controls = GalleryState::new();
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-controls-dark.png",
        &mut controls,
    )?;

    let mut controls_light = GalleryState::new();
    controls_light.update(GalleryMessage::SetTheme(ThemeAppearance::Light));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-controls-light.png",
        &mut controls_light,
    )?;
    // (180, 60) is the band the section label sits in, and nothing there is
    // hit-testable, so this hovered nothing and came out byte-identical to
    // `gallery-controls`. It is also named for tools this Gallery's sidebar
    // does not have — no call site anywhere passes `SidebarSection::tools` —
    // so the name moves to what the scene can actually show: a sidebar row
    // hovered inside the full shell. (12, 104, 200x28) is the "表面" row.
    gallery_snapshot_with_cursor(
        &mut snapshots,
        &mut recorder,
        "gallery-sidebar-hover-dark.png",
        &mut controls,
        LogicalPoint::new(100.0, 118.0),
    )?;
    gallery_snapshot_with_cursor(
        &mut snapshots,
        &mut recorder,
        "gallery-sidebar-hover-light.png",
        &mut controls_light,
        LogicalPoint::new(100.0, 118.0),
    )?;

    let mut loading = GalleryState::new();
    loading.update(GalleryMessage::ToggleLoading);
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-loading-dark.png",
        &mut loading,
    )?;

    let mut surfaces = GalleryState::new();
    surfaces.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-surfaces-dark.png",
        &mut surfaces,
    )?;

    let mut surfaces_light = GalleryState::new();
    surfaces_light.update(GalleryMessage::SetTheme(ThemeAppearance::Light));
    surfaces_light.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-surfaces-light.png",
        &mut surfaces_light,
    )?;

    surfaces.update(GalleryMessage::PaneChrome(
        nana_ui::runtime::PaneChromeActionKind::SplitHorizontal,
    ));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-surfaces-split-dark.png",
        &mut surfaces,
    )?;

    surfaces_light.update(GalleryMessage::PaneChrome(
        nana_ui::runtime::PaneChromeActionKind::SplitHorizontal,
    ));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-surfaces-split-light.png",
        &mut surfaces_light,
    )?;

    let mut cards = GalleryState::new();
    cards.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    cards.update(GalleryMessage::SelectSurfaceView(SurfaceView::Cards));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-cards-dark.png",
        &mut cards,
    )?;

    surfaces_light.update(GalleryMessage::SelectSurfaceView(SurfaceView::Cards));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-cards-light.png",
        &mut surfaces_light,
    )?;

    let mut feedback = GalleryState::new();
    feedback.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-feedback-dark.png",
        &mut feedback,
    )?;

    let mut rich_text = GalleryState::new();
    rich_text.update(GalleryMessage::SelectSection(GallerySection::RichText));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-rich-text-dark.png",
        &mut rich_text,
    )?;
    rich_text.update(GalleryMessage::SetTheme(ThemeAppearance::Light));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-rich-text-light.png",
        &mut rich_text,
    )?;

    let mut popover = GalleryState::new();
    popover.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    popover.update(GalleryMessage::TogglePopover);
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-popover-dark.png",
        &mut popover,
    )?;

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
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-context-menu-dark.png",
        &mut context_menu,
    )?;
    context_menu.update(GalleryMessage::SetTheme(ThemeAppearance::Light));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-context-menu-light.png",
        &mut context_menu,
    )?;

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
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-context-menu-search-dark.png",
        &mut context_menu_search,
    )?;

    let mut context_menu_search_light = GalleryState::new();
    context_menu_search_light.update(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
        width: GALLERY_SIZE.width as f32,
        height: GALLERY_SIZE.height as f32,
    }));
    context_menu_search_light.update(GalleryMessage::SetTheme(ThemeAppearance::Light));
    context_menu_search_light.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    context_menu_search_light.update(GalleryMessage::ToggleContextMenu);
    context_menu_search_light.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::Search("copy".to_owned()),
    ));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-context-menu-search-light.png",
        &mut context_menu_search_light,
    )?;

    let mut command_palette = GalleryState::new();
    command_palette.update(GalleryMessage::ToggleCommandPalette);
    command_palette.update(GalleryMessage::CommandPalette(CommandPaletteEvent::Search(
        "工作区".to_owned(),
    )));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-command-palette-dark.png",
        &mut command_palette,
    )?;

    let mut command_palette_light = GalleryState::new();
    command_palette_light.update(GalleryMessage::SetTheme(ThemeAppearance::Light));
    command_palette_light.update(GalleryMessage::ToggleCommandPalette);
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-command-palette-light.png",
        &mut command_palette_light,
    )?;

    let mut dialog = GalleryState::new();
    dialog.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    dialog.update(GalleryMessage::ToggleDialog);
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-dialog-dark.png",
        &mut dialog,
    )?;

    let mut image_viewer = GalleryState::new();
    image_viewer.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    image_viewer.update(GalleryMessage::ToggleImageViewer);
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-image-viewer-dark.png",
        &mut image_viewer,
    )?;

    let mut workspace = GalleryState::new();
    workspace.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-workspace-dark.png",
        &mut workspace,
    )?;

    // There used to be a `gallery-workspace-dock-preview` pair here, built
    // from exactly the same two messages as the scene above and therefore
    // byte-identical to it in dark. No drag was ever started, and `GalleryDock`
    // has no message that would start one. The five `dock-preview-*` snapshots
    // already cover every drop zone, so the dark copy is gone and the light one
    // keeps the coverage it was really providing, under its real name.
    let mut workspace_light = GalleryState::new();
    workspace_light.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    workspace_light.update(GalleryMessage::SetTheme(ThemeAppearance::Light));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-workspace-light.png",
        &mut workspace_light,
    )?;

    let mut sidebar_collapsed = GalleryState::new();
    sidebar_collapsed.update(GalleryMessage::Workspace(
        WorkspaceAction::SetRegionCollapsed(nana_ui::RegionId::Resources, true),
    ));
    sidebar_collapsed.update(GalleryMessage::Workspace(WorkspaceAction::AnimationFrame(
        std::time::Duration::from_millis(300),
    )));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-sidebar-collapsed-dark.png",
        &mut sidebar_collapsed,
    )?;

    let mut settings = GalleryState::new();
    settings.update(GalleryMessage::OpenSettings);
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-settings-appearance-dark.png",
        &mut settings,
    )?;

    settings.update(GalleryMessage::SetTheme(ThemeAppearance::Light));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-settings-appearance-light.png",
        &mut settings,
    )?;

    settings.update(GalleryMessage::SetTheme(ThemeAppearance::Dark));
    settings.update(GalleryMessage::SelectSettingsTab(SettingsTabId::from(
        "workspace",
    )));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-settings-workspace-dark.png",
        &mut settings,
    )?;

    settings.update(GalleryMessage::SelectSettingsTab(SettingsTabId::from(
        "about",
    )));
    gallery_snapshot(
        &mut snapshots,
        &mut recorder,
        "gallery-settings-about-dark.png",
        &mut settings,
    )?;

    recorder.finish()
}

#[derive(Debug, Clone, Copy)]
struct MigrationLayoutMessage {
    component: &'static str,
    bounds: LogicalRect,
}

fn component_migration_snapshots(
    snapshots: &mut OffscreenSnapshots,
    recorder: &mut Recorder,
    theme: ThemeAppearance,
) -> Result<(), Box<dyn std::error::Error>> {
    let (runtime_document, runtime_layout) = migration_runtime_document(theme)?;
    let clear = clear_color(theme);
    let pixels = snapshots.paint(runtime_document.scene(), MIGRATION_SIZE, clear, None, None)?;
    let key = "migration-first-batch-dark.png";
    recorder.record(key, MIGRATION_SIZE, &pixels, clear)?;
    write_migration_layout_report(&recorder.sibling(key, "layout.txt"), &runtime_layout)
}

fn migration_runtime_document(
    theme: ThemeAppearance,
) -> Result<(RuntimeDocument, Vec<MigrationLayoutMessage>), Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(2).expect("migration fixture document ID is non-zero");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_preset_theme(theme)?;
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
    let (_, (title, input, button, checkbox)) =
        document.context_mut().mount_view_root(document_id, || {
            let refs = (entity_ref(), entity_ref(), entity_ref(), entity_ref());
            let (title, input, button, checkbox) = refs;
            with_refs(
                widget(RuntimeList::new().style(root_style))
                    .key("root")
                    .children((
                        widget(RuntimeText::new("Migration fixture").style(NodeStyle {
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
                        }))
                        .key("title")
                        .entity_ref(title),
                        widget(RuntimeTextInput::new("release/issue-7").label("Branch"))
                            .key("input")
                            .entity_ref(input),
                        widget(RuntimeButton::new("Run build").kind(ButtonKind::Primary))
                            .key("button")
                            .entity_ref(button),
                        widget(RuntimeCheckbox::new("Notifications", true))
                            .key("checkbox")
                            .entity_ref(checkbox),
                    )),
                refs,
            )
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

pub(crate) fn side_by_side(left: &[u8], right: &[u8], size: Size<u32>, gap: u32) -> Vec<u8> {
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

pub(crate) fn pixel_difference(left: &[u8], right: &[u8]) -> Vec<u8> {
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
    let mut report = String::from("component\truntime_bounds\n");
    for entry in runtime {
        report.push_str(&format!("{}\t{:?}\n", entry.component, entry.bounds));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, report)?;
    Ok(())
}

fn runtime_scene_snapshot(
    snapshots: &mut OffscreenSnapshots,
    recorder: &mut Recorder,
    name: &str,
    theme: ThemeAppearance,
) -> Result<(), Box<dyn std::error::Error>> {
    let size = Size::new(900, 500);
    let document = runtime_scene_document(theme)?;
    let clear = clear_color(theme);
    let pixels = snapshots.paint(document.scene(), size, clear, None, None)?;
    recorder.record(name, size, &pixels, clear)
}

fn runtime_scene_document(
    theme: ThemeAppearance,
) -> Result<RuntimeDocument, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(1).expect("snapshot document ID is non-zero");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_preset_theme(theme)?;
    let slider_component = RuntimeRangeField::new(68.0, 0.0, 100.0, 1.0).label("Volume");
    let rows = [
        ["Build", "Status", "Duration"],
        ["#1042", "Succeeded", "1m 18s"],
        ["#1041", "Succeeded", "1m 21s"],
        ["#1040", "Failed", "42s"],
    ];
    let (_, refs) = document.context_mut().mount_view_root(document_id, || {
        let (title, input, button, table) =
            (entity_ref(), entity_ref(), entity_ref(), entity_ref());
        let (checkbox, toggle, slider, tabs) =
            (entity_ref(), entity_ref(), entity_ref(), entity_ref());
        let (activity, card, add_source, notes) =
            (entity_ref(), entity_ref(), entity_ref(), entity_ref());
        let activity_lines: [_; 3] = std::array::from_fn(|_| entity_ref());
        let source_list = entity_ref();
        let source_items: [_; 3] = std::array::from_fn(|_| entity_ref());
        let mut cells = Vec::new();
        let mut row_views = Vec::new();
        for (row_index, values) in rows.into_iter().enumerate() {
            let mut cell_views = Vec::new();
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
                let cell = entity_ref::<RuntimeTableCell>();
                cells.push(cell);
                cell_views.push(
                    widget(
                        RuntimeTableCell::new(value)
                            .column_header(row_index == 0)
                            .style(style),
                    )
                    .key(format!("cell-{column}"))
                    .entity_ref(cell),
                );
            }
            row_views.push(
                widget(RuntimeTableRow::new())
                    .key(format!("row-{row_index}"))
                    .children(cell_views),
            );
        }
        let view = (
            widget(RuntimeText::new("Build queue").style(NodeStyle {
                foreground: Some(SemanticColorRole::Text),
                layout: Arc::new(LayoutStyle {
                    font_size: Some(20.0),
                    font_weight: Some(600),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            }))
            .key("title")
            .entity_ref(title),
            widget(RuntimeTextInput::new("release/issue-7").label("Branch"))
                .key("input")
                .entity_ref(input),
            widget(RuntimeButton::new("Run build"))
                .key("button")
                .entity_ref(button),
            widget(RuntimeTable::new().label("Recent builds"))
                .key("table")
                .entity_ref(table)
                .children(row_views),
            widget(RuntimeCheckbox::new("Notifications", true))
                .key("checkbox")
                .entity_ref(checkbox),
            widget(RuntimeSwitch::new("Auto build", true))
                .key("toggle")
                .entity_ref(toggle),
            widget(slider_component).key("slider").entity_ref(slider),
            widget(RuntimeTabs::new("preview").label("Output").options([
                RuntimeTabOption::new("preview", "Preview"),
                RuntimeTabOption::new("program", "Program"),
            ]))
            .key("tabs")
            .entity_ref(tabs),
            widget(
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
            )
            .key("activity")
            .entity_ref(activity)
            .children((
                widget(RuntimeText::new("Queued #1043"))
                    .key("queued")
                    .entity_ref(activity_lines[0]),
                widget(RuntimeText::new("Built #1042"))
                    .key("built")
                    .entity_ref(activity_lines[1]),
                widget(RuntimeText::new("Published artifacts"))
                    .key("published")
                    .entity_ref(activity_lines[2]),
            )),
            widget(RuntimeCard::new().label("Source inspector"))
                .key("card")
                .entity_ref(card)
                .children((
                    widget(RuntimeIconButton::new(nana_ui::Icon::Add, "Add source"))
                        .key("add-source")
                        .entity_ref(add_source),
                    widget(
                        RuntimeTextArea::new("Camera follows Program.\nAudio monitoring enabled.")
                            .label("Source notes"),
                    )
                    .key("notes")
                    .entity_ref(notes),
                    widget(RuntimeList::new().label("Scene sources"))
                        .key("sources")
                        .entity_ref(source_list)
                        .children((
                            widget(RuntimeListItem::new("Camera").selected(true))
                                .key("camera")
                                .entity_ref(source_items[0]),
                            widget(RuntimeListItem::new("Live2D actor"))
                                .key("actor")
                                .entity_ref(source_items[1]),
                            widget(RuntimeListItem::new("Lower third").disabled(true))
                                .key("lower-third")
                                .entity_ref(source_items[2]),
                        )),
                )),
        );
        with_refs(
            view,
            (
                (title, input, button, table, checkbox, toggle, slider, tabs),
                (activity, activity_lines, card, add_source, notes),
                (source_list, source_items, cells),
            ),
        )
    })?;
    let (
        (title, input, button, table, checkbox, toggle, slider, tabs),
        (activity, activity_lines, card, add_source, notes),
        (source_list, source_items, cells),
    ) = refs;
    let cells = cells.into_iter().map(|cell| cell.stable_id());
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
    recorder: &mut Recorder,
    name: &str,
    theme: ThemeAppearance,
    chrome: WindowChrome,
    hover: Option<LogicalPoint>,
) -> Result<(), Box<dyn std::error::Error>> {
    let size = Size::new(900, 120);
    let mut document = titlebar_document(theme, chrome)?;
    if let Some(point) = hover {
        dispatch_pointer(&mut document, size, PointerPhase::Move, point)?;
    }
    let clear = clear_color(theme);
    let pixels = snapshots.paint(document.scene(), size, clear, None, None)?;
    recorder.record(name, size, &pixels, clear)
}

fn titlebar_document(
    theme: ThemeAppearance,
    chrome: WindowChrome,
) -> Result<RuntimeDocument, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(3).expect("titlebar document");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_preset_theme(theme)?;
    let native = !chrome.uses_custom_controls();
    document.context_mut().mount_view_root(document_id, || {
        widget(
            AppTitleBar::new("NanaUI")
                .center_width(420.0)
                .native_controls(native)
                .show_window_controls(true),
        )
        .leading(widget(labeled_text(
            "NANA",
            SemanticColorRole::Accent,
            12.0,
            600,
        )))
        .center(widget(labeled_text(
            "LiliaCode › 恢复 Native 侧边栏交互与主界面布局",
            SemanticColorRole::Text,
            13.0,
            400,
        )))
        .trailing(widget(labeled_text(
            "Gallery",
            SemanticColorRole::Muted,
            type_scale::HINT,
            type_scale::REGULAR,
        )))
        .child_slot(title_bar_controls(native), AppTitleBar::controls)
    })?;
    document.flush(
        LayoutViewport::new(900.0, 120.0),
        &mut NanaTextShaper::default(),
    )?;
    Ok(document)
}

fn dock_window_snapshot(
    snapshots: &mut OffscreenSnapshots,
    recorder: &mut Recorder,
    name: &str,
    theme: ThemeAppearance,
    chrome: WindowChrome,
    root: DockNode,
) -> Result<(), Box<dyn std::error::Error>> {
    let size = if matches!(
        name,
        n if n.contains("merged")
    ) {
        Size::new(520, 360)
    } else {
        Size::new(420, 320)
    };
    let document = dock_window_document(theme, chrome, root, size)?;
    let clear = clear_color(theme);
    let pixels = snapshots.paint(document.scene(), size, clear, None, None)?;
    recorder.record(name, size, &pixels, clear)
}

fn dock_window_document(
    theme: ThemeAppearance,
    chrome: WindowChrome,
    root: DockNode,
    size: Size<u32>,
) -> Result<RuntimeDocument, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(4).expect("dock window document");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_preset_theme(theme)?;
    // Tab chrome only: pane text overlaps titles at this fixture size.
    let dock = RuntimeDock::new(root)
        .title("navigation", "导航")
        .title("console", "控制台")
        .title("output", "输出")
        .title("editor", "Editor");
    let native = !chrome.uses_custom_controls();
    document.context_mut().mount_view_root(document_id, || {
        widget(AppShell::new())
            .title_bar(
                widget(
                    AppTitleBar::new("NanaUI Gallery")
                        .native_controls(native)
                        .show_window_controls(true),
                )
                .child_slot(title_bar_controls(native), AppTitleBar::controls),
            )
            .body(widget(dock))
    })?;
    document.flush(
        LayoutViewport::new(size.width as f32, size.height as f32),
        &mut NanaTextShaper::default(),
    )?;
    Ok(document)
}

fn dock_drag_window_snapshot(
    snapshots: &mut OffscreenSnapshots,
    recorder: &mut Recorder,
    name: &str,
    theme: ThemeAppearance,
    chrome: WindowChrome,
) -> Result<(), Box<dyn std::error::Error>> {
    let size = Size::new(420, 240);
    let document = dock_window_document(theme, chrome, DockNode::item("navigation", None), size)?;
    let clear = clear_color(theme);
    let pixels = snapshots.paint(document.scene(), size, clear, None, None)?;
    recorder.record(name, size, &pixels, clear)
}

fn dock_preview_snapshot(
    snapshots: &mut OffscreenSnapshots,
    recorder: &mut Recorder,
    name: &str,
    theme: ThemeAppearance,
    zone: DockDropZone,
    outside: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let size = Size::new(420, 240);
    let document = dock_preview_document(theme, zone, outside, size)?;
    let clear = clear_color(theme);
    let pixels = snapshots.paint(document.scene(), size, clear, None, None)?;
    recorder.record(name, size, &pixels, clear)
}

fn dock_preview_document(
    theme: ThemeAppearance,
    zone: DockDropZone,
    outside: bool,
    size: Size<u32>,
) -> Result<RuntimeDocument, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(5).expect("dock preview document");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_preset_theme(theme)?;
    let drop = if outside {
        None
    } else {
        let target = match zone {
            DockDropZone::Left | DockDropZone::Top => "source",
            DockDropZone::Right | DockDropZone::Bottom | DockDropZone::Tab => "editor",
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
    recorder: &mut Recorder,
    name: &str,
    state: &mut GalleryState,
) -> Result<(), Box<dyn std::error::Error>> {
    state.flush_snapshot_scene();
    let clear = clear_color(state.preset_theme());
    let pixels = paint_gallery(snapshots, state, GALLERY_SIZE)?;
    recorder.record(name, GALLERY_SIZE, &pixels, clear)
}

fn gallery_snapshot_with_cursor(
    snapshots: &mut OffscreenSnapshots,
    recorder: &mut Recorder,
    name: &str,
    state: &mut GalleryState,
    cursor: LogicalPoint,
) -> Result<(), Box<dyn std::error::Error>> {
    state.flush_snapshot_scene();
    state.snapshot_hover(cursor.x, cursor.y);
    state.snapshot_settle(nana_ui_core::motion::HOVER_COLOR);
    gallery_snapshot(snapshots, recorder, name, state)
}

fn paint_gallery(
    snapshots: &mut OffscreenSnapshots,
    state: &GalleryState,
    size: Size<u32>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let clear = clear_color(state.preset_theme());
    let colors = state.preset_theme().palette();
    let gpu = gpu::create_snapshot_gpu(&snapshots.gpu, colors.background, colors.accent_strong);
    // The Gallery's ready thumbnail is a demo HostTexture slot. Snapshot hosts
    // must populate it just like the standalone GPU fixtures.
    let binding = gpu
        .textures
        .get(gpu::SNAPSHOT_GPU_SLOT)
        .expect("snapshot texture");
    gpu.textures.register(
        "gallery.thumb",
        binding.texture,
        binding.width,
        binding.height,
        binding.alpha_mode,
    );
    match state.active_scene() {
        Some(scene) => snapshots.paint(
            scene,
            size,
            clear,
            Some(&gpu.textures),
            Some(&gpu.renderers),
        ),
        None => snapshots.paint_layers(&[], size, clear, Some(&gpu.textures), Some(&gpu.renderers)),
    }
}

/// sRGB on purpose: `OffscreenSnapshots` converts it to linear itself, and
/// `Recorder::record` compares it against sRGB pixels.
fn clear_color(theme: ThemeAppearance) -> [f32; 4] {
    let color = theme.palette().background;
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

/// A title bar's controls slot for one window chrome: custom Minimize /
/// Maximize / Close buttons, or the empty placeholder the platform's own
/// buttons are moved onto.
///
/// The bar that adopts this keeps `show_window_controls` true either way.
/// Turning it off hides the placeholder, and with it the leading band the
/// native buttons live in — the native fixture would then lay out exactly
/// like the custom one, which is what these snapshots exist to tell apart.
fn title_bar_controls(native: bool) -> AnyView {
    if native {
        // The placeholder stands for buttons it does not own, so it stays
        // childless: `AppTitleBarControls` projects no children in this mode,
        // and any mounted here would just paint inside the band.
        return widget(AppTitleBarControls::new(false).native(true)).into_any();
    }
    widget(AppTitleBarControls::new(false).native(false))
        .child_slot(
            widget(window_control(Icon::Minimize, "Minimize")),
            AppTitleBarControls::minimize,
        )
        .child_slot(
            widget(window_control(Icon::Maximize, "Maximize")),
            AppTitleBarControls::maximize,
        )
        .child_slot(
            widget(window_control(Icon::Close, "Close")),
            AppTitleBarControls::close,
        )
        .into_any()
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
    nana_ui::HeadlessInput::bind(document.context_mut(), document_id).route(
        document.context_mut(),
        InputPayload::Pointer(PointerInput {
            buttons: 0,
            pressure: 0.5,
            ..PointerInput::mouse(phase, point.x, point.y)
        }),
    )?;
    document.flush(
        LayoutViewport::new(size.width as f32, size.height as f32),
        &mut NanaTextShaper::default(),
    )?;
    Ok(())
}
