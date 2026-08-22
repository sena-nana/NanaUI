use super::{
    ContextAction, GalleryApp, GalleryContextMenuEvent, GalleryDock, GalleryMessage,
    GalleryOverlay, GallerySection, GalleryState, SurfaceView,
};
use crate::runtime_host::RuntimeSceneInput;
use crate::runtime_settings::SettingsRuntimeInput;
use nana_ui::LogicalPoint;
use nana_ui::PaneChromeActionKind;
use nana_ui::window_chrome::{WindowChromeAction, WindowChromeEvent, WindowChromeState};
use nana_ui::{
    ActionId, ActionPickerNavigation, AppearanceSettings, BackdropTarget, CommandPaletteEvent,
    DockWorkspaceEvent, Icon, KeyModifiers, KeyStroke, MaterialOutcome, RegionId, SelectionMove,
    SettingsTabId, SplitPaneAction, ThemeMode, TreeViewEvent, WindowMaterialMode, WorkspaceAction,
};
use nana_ui_platform::WindowCommand;

#[test]
fn gallery_interactions_update_real_state() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::PrimaryAction);
    state.update(GalleryMessage::ToggleLoading);
    state.update(GalleryMessage::InputChanged("Field".to_owned()));
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    state.update(GalleryMessage::ToggleContextMenu);
    state.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::Select("rename".into()),
    ));

    assert_eq!(state.primary_clicks, 1);
    assert!(state.loading);
    assert_eq!(state.input, "Field");
    assert_eq!(state.section, GallerySection::Feedback);
    assert!(!state.overlay.is_open());
    assert_eq!(state.context_action, Some(ContextAction::Rename));
}

#[test]
fn gallery_overlays_paint_a_runtime_scene() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::ToggleCommandPalette);
    assert!(state.overlay.contains(&GalleryOverlay::CommandPalette));
    assert!(state.overlay_shares_active_document());
    assert!(state.gallery_overlay_runtime_scene_populated());
    assert!(
        state
            .runtime_document()
            .is_some_and(|document| !document.scene().is_empty())
    );

    state.update(GalleryMessage::ToggleDialog);
    assert!(state.overlay.contains(&GalleryOverlay::Dialog));
    assert!(!state.overlay.contains(&GalleryOverlay::CommandPalette));
    assert!(state.gallery_overlay_runtime_scene_populated());
    assert!(
        state
            .runtime_document()
            .is_some_and(|document| !document.scene().is_empty())
    );

    state.update(GalleryMessage::ToggleImageViewer);
    assert!(state.overlay.contains(&GalleryOverlay::ImageViewer));
    assert!(state.gallery_overlay_runtime_scene_populated());
    assert!(
        state
            .runtime_document()
            .is_some_and(|document| !document.scene().is_empty())
    );

    state.update(GalleryMessage::ToggleContextMenu);
    assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
    assert!(state.gallery_overlay_runtime_scene_populated());
    assert!(
        state
            .runtime_document()
            .is_some_and(|document| !document.scene().is_empty())
    );

    state.update(GalleryMessage::DismissOverlay);
    assert!(!state.overlay.is_open());
    assert!(!state.gallery_overlay_runtime_scene_populated());
}

#[test]
fn gallery_overlays_are_mutually_exclusive_and_dismissible() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::ToggleDialog);
    assert!(state.overlay.contains(&GalleryOverlay::Dialog));

    state.update(GalleryMessage::ToggleContextMenu);
    assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
    assert!(!state.overlay.contains(&GalleryOverlay::Dialog));

    state.update(GalleryMessage::DismissOverlay);
    assert!(!state.overlay.is_open());
}

#[test]
fn destructive_menu_action_requires_confirmation() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::SelectListItem(2));
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    state.update(GalleryMessage::ToggleContextMenu);

    state.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::Select("remove".into()),
    ));
    assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
    assert_eq!(
        state.menu_confirmation.pending(),
        Some(&ContextAction::Remove)
    );
    assert_eq!(state.selected_item, 2);

    state.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::Select("remove".into()),
    ));
    assert!(!state.overlay.is_open());
    assert_eq!(state.context_action, Some(ContextAction::Remove));
    assert_eq!(state.selected_item, 0);
}

#[test]
fn dialog_confirmation_executes_and_closes_the_overlay() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::ToggleDialog);
    assert!(state.overlay.contains(&GalleryOverlay::Dialog));
    assert_eq!(
        state.gallery_overlay_dialog_copy(),
        Some(("确认操作".to_owned(), "此操作会更新当前状态".to_owned()))
    );

    state.update(GalleryMessage::ConfirmDialog);
    assert!(!state.overlay.is_open());
    assert_eq!(state.confirmed_actions, 1);
}

#[test]
fn image_viewer_mounts_an_accent_preview_child() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::ToggleImageViewer);
    assert!(state.overlay.contains(&GalleryOverlay::ImageViewer));
    let (has_child, labels) = state
        .gallery_overlay_image_preview()
        .expect("image viewer remains mounted");
    assert!(has_child);
    assert!(labels.iter().any(|label| label == "NANA"));
    assert!(labels.iter().any(|label| label == "完整组件库"));
}

#[test]
fn context_menu_maps_leaf_suffixes_and_keeps_item_icons() {
    assert_eq!(
        crate::runtime_overlays::gallery_context_action_from_value("project"),
        None
    );
    assert_eq!(
        crate::runtime_overlays::gallery_context_action_from_value("duplicate"),
        Some(ContextAction::Duplicate)
    );
    assert_eq!(
        crate::runtime_overlays::gallery_context_action_from_value("project/duplicate"),
        Some(ContextAction::Duplicate)
    );
    assert_eq!(
        crate::runtime_overlays::gallery_context_action_from_value("rename"),
        Some(ContextAction::Rename)
    );
    assert_eq!(
        crate::runtime_overlays::gallery_context_action_from_value("project/rename"),
        Some(ContextAction::Rename)
    );
    assert_eq!(
        crate::runtime_overlays::gallery_context_action_from_value("remove"),
        Some(ContextAction::Remove)
    );

    let mut state = GalleryState::new();
    let items = crate::runtime_overlays::gallery_runtime_context_item_icons(state.context_items());
    assert!(
        items.iter().any(|(value, label, icon)| {
            value == "project/duplicate" && label == "复制项目" && *icon == Some(Icon::Add)
        }),
        "copy leaf must keep Icon::Add: {items:?}"
    );
    assert!(
        items.iter().any(|(value, label, icon)| {
            value == "project/rename" && label == "重命名项目" && *icon == Some(Icon::File)
        }),
        "rename leaf must keep Icon::File: {items:?}"
    );
    assert!(
        items
            .iter()
            .any(|(value, _, icon)| value == "project" && icon.is_none()),
        "parent path must not be a leaf action: {items:?}"
    );

    state.update(GalleryMessage::ToggleContextMenu);
    state.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::Select("rename".into()),
    ));
    assert_eq!(state.context_action, Some(ContextAction::Rename));
}

#[test]
fn segmented_surface_view_supports_click_and_roving_selection() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::SelectSurfaceView(SurfaceView::Cards));
    assert_eq!(state.surface_selection.selected(), 1);

    state.update(GalleryMessage::SelectSurfaceCard(1));
    assert_eq!(state.selected_surface_card, 1);

    state.update(GalleryMessage::NavigateSurfaceView(SelectionMove::Next));
    assert_eq!(state.surface_selection.selected(), 0);
    state.update(GalleryMessage::NavigateSurfaceView(SelectionMove::Last));
    assert_eq!(state.surface_selection.selected(), 1);
}

#[test]
fn search_dropdown_input_updates_tracked_query() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::SearchDropdownInput("Beta".to_owned()));
    assert_eq!(state.search_dropdown_query, "Beta");
}

#[test]
fn context_menu_search_filters_items() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    state.update(GalleryMessage::ToggleContextMenu);
    state.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::Search("重命名".to_owned()),
    ));
    assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
    assert_eq!(state.context_query, "重命名");
}

#[test]
fn context_menu_tracks_hovered_submenu_paths_without_closing_the_overlay() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    state.update(GalleryMessage::ToggleContextMenu);
    state.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::OpenSubmenu(vec![0]),
    ));

    assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
    assert_eq!(state.context_path, vec![0]);

    state.update(GalleryMessage::ContextMenu(
        GalleryContextMenuEvent::OpenSubmenu(Vec::new()),
    ));
    assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
    assert!(state.context_path.is_empty());
}

#[test]
fn tree_view_events_update_expansion_and_selection_by_stable_id() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::TreeView(TreeViewEvent::Toggle(
        "src".to_owned(),
    )));
    assert!(!state.tree_expanded);
    state.update(GalleryMessage::TreeView(TreeViewEvent::Select(
        "README.md".to_owned(),
    )));
    assert_eq!(state.tree_selected, "README.md");
}

#[test]
fn pane_chrome_actions_change_the_real_gallery_pane_state() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::PaneChrome(
        PaneChromeActionKind::SplitHorizontal,
    ));
    assert!(state.pane_chrome_split);
    state.update(GalleryMessage::PaneChrome(PaneChromeActionKind::CloseItem));
    assert!(!state.pane_chrome_item_open);
    assert!(!state.pane_chrome_split);
}

#[test]
fn command_palette_keeps_title_and_search_query_after_search() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::ToggleCommandPalette);
    state.update(GalleryMessage::CommandPalette(CommandPaletteEvent::Search(
        "工作区".to_owned(),
    )));

    assert_eq!(state.action_picker.query(), "工作区");
    let (title, query, input, selected) = state
        .gallery_overlay_command_palette_state()
        .expect("command palette remains mounted");
    assert_eq!(title, "命令");
    assert_eq!(query, "工作区");
    assert_eq!(input, "工作区");
    assert_eq!(selected, 0);
    assert_eq!(
        state.gallery_overlay_command_palette_visual(),
        Some(("命令".to_owned(), "工作区".to_owned()))
    );
    assert!(
        state
            .palette_items()
            .iter()
            .all(|item| item.category.as_deref() == Some("工作区"))
    );
}

#[test]
fn command_palette_filters_by_context_and_dispatches_real_actions() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::ToggleCommandPalette);
    assert!(state.overlay.contains(&GalleryOverlay::CommandPalette));
    assert!(
        state
            .palette_items()
            .iter()
            .all(|item| item.action.as_str() != "graph.reset_viewport")
    );

    state.update(GalleryMessage::CommandPalette(CommandPaletteEvent::Select(
        ActionId::from("appearance.toggle_theme"),
    )));
    assert_eq!(state.theme, ThemeMode::Light);
    assert!(!state.overlay.is_open());

    state.update(GalleryMessage::SelectSection(GallerySection::Graph));
    state.update(GalleryMessage::ToggleCommandPalette);
    assert!(
        state
            .palette_items()
            .iter()
            .any(|item| item.action.as_str() == "graph.reset_viewport")
    );
}

#[test]
fn command_palette_keybinding_opens_and_keyboard_navigation_selects() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::KeyStroke(KeyStroke::new(
        "p",
        KeyModifiers::primary().with_shift(),
    )));
    assert!(state.overlay.contains(&GalleryOverlay::CommandPalette));

    state.update(GalleryMessage::NavigateCommandPalette(
        ActionPickerNavigation::Next,
    ));
    assert_eq!(state.action_picker.selected(), 1);
    state.update(GalleryMessage::NavigateCommandPalette(
        ActionPickerNavigation::Confirm,
    ));
    assert_eq!(
        state.palette_action.as_ref().map(ActionId::as_str),
        Some("appearance.toggle_theme")
    );
    assert_eq!(state.theme, ThemeMode::Light);
    assert!(!state.overlay.is_open());
}

#[test]
fn loading_state_blocks_until_the_async_cycle_finishes() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::ToggleLoading);
    assert!(state.loading);

    for _ in 0..11 {
        state.update(GalleryMessage::LoadingTick);
        assert!(state.loading);
    }
    state.update(GalleryMessage::LoadingTick);
    assert!(!state.loading);
    assert_eq!(state.loading_ticks, 0);
}

#[test]
fn selection_and_edit_switches_control_editor_availability() {
    let mut state = GalleryState::new();
    assert!(state.editor_enabled());

    state.update(GalleryMessage::ToggleCheck(false));
    assert!(!state.editor_enabled());

    state.update(GalleryMessage::ToggleCheck(true));
    state.update(GalleryMessage::ToggleSwitch(false));
    assert!(!state.editor_enabled());
}

#[test]
fn workspace_section_controls_auxiliary_regions_without_losing_sizes() {
    let mut state = GalleryState::new();
    for id in [
        RegionId::PrimaryToolbar,
        RegionId::Inspector,
        RegionId::Diagnostics,
    ] {
        assert!(
            state
                .workspace
                .layout()
                .region(&id)
                .expect("gallery region")
                .hidden_value()
        );
    }

    state.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    state.update(GalleryMessage::Workspace(WorkspaceAction::SetRegionSize(
        RegionId::Inspector,
        340.0,
    )));
    assert!(
        !state
            .workspace
            .layout()
            .region(&RegionId::Inspector)
            .expect("inspector")
            .hidden_value()
    );

    state.update(GalleryMessage::SelectSection(GallerySection::Controls));
    assert!(
        state
            .workspace
            .layout()
            .region(&RegionId::Inspector)
            .expect("inspector")
            .hidden_value()
    );

    state.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    let inspector = state
        .workspace
        .layout()
        .region(&RegionId::Inspector)
        .expect("inspector");
    assert!(!inspector.hidden_value());
    assert_eq!(inspector.size_value(), Some(340.0));
}

#[test]
fn settings_return_to_the_gallery_and_appearance_updates_immediately() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    state.update(GalleryMessage::OpenSettings);
    assert!(state.settings_open);

    state.update(GalleryMessage::SetTheme(ThemeMode::Light));
    state.update(GalleryMessage::SetStandardRadius(8));
    state.update(GalleryMessage::BackFromSettings);

    assert!(!state.settings_open);
    assert_eq!(state.section, GallerySection::Surfaces);
    assert_eq!(state.theme_mode(), ThemeMode::Light);
    assert_eq!(state.appearance.standard_radius(), 8.0);
}

#[test]
fn gallery_runtime_assembles_markdown_fence_children() {
    let mut state = GalleryState::new();
    assert!(state.gallery_runtime_markdown_has_mermaid_presenter());
    state.update(GalleryMessage::SelectSection(GallerySection::RichText));
    assert!(state.gallery_runtime_markdown_has_mermaid_presenter());
}

#[test]
fn gallery_main_route_paints_a_runtime_scene() {
    let mut state = GalleryState::new();
    assert!(!state.settings_open);
    assert!(state.gallery_runtime_scene_populated());
    assert!(
        state
            .runtime_document()
            .is_some_and(|document| !document.scene().is_empty())
    );
    state.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
    assert!(state.gallery_runtime_scene_populated());
    assert!(
        state
            .runtime_document()
            .is_some_and(|document| !document.scene().is_empty())
    );
}

#[test]
fn settings_route_paints_a_runtime_scene() {
    let mut state = GalleryState::new();
    assert!(!state.settings_runtime_scene_populated());
    state.update(GalleryMessage::OpenSettings);
    assert!(state.settings_open);
    assert!(state.settings_runtime_scene_populated());
    assert!(
        state
            .runtime_document()
            .is_some_and(|document| !document.scene().is_empty())
    );
    state.update(GalleryMessage::SelectSettingsTab(SettingsTabId::from(
        "workspace",
    )));
    assert!(state.settings_runtime_scene_populated());
    assert!(
        state
            .runtime_document()
            .is_some_and(|document| !document.scene().is_empty())
    );
    state.update(GalleryMessage::BackFromSettings);
    assert!(!state.settings_open);
}

#[test]
fn settings_runtime_title_bar_chrome_starts_window_drag() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::OpenSettings);
    assert!(state.settings_open);
    assert!(state.settings_runtime_scene_populated());

    let origin = LogicalPoint::new(640.0, 18.0);
    let dragged = LogicalPoint::new(648.0, 18.0);
    state.update(GalleryMessage::SettingsRuntime(
        SettingsRuntimeInput::PointerDown {
            button: 0,
            point: origin,
        },
    ));
    let mut chrome = WindowChromeState::default();
    let mut action = None;
    for event in state.take_settings_window_chrome_events() {
        assert!(
            matches!(
                event,
                WindowChromeEvent::PointerMoved(_) | WindowChromeEvent::PointerPressed
            ),
            "title-bar chrome must not emit control actions on an empty press: {event:?}"
        );
        action = chrome.update(event).or(action);
    }
    state.update(GalleryMessage::SettingsRuntime(
        SettingsRuntimeInput::PointerMove(dragged),
    ));
    for event in state.take_settings_window_chrome_events() {
        action = chrome.update(event).or(action);
    }

    assert_eq!(action, Some(WindowChromeAction::Drag));
}

#[test]
fn gallery_title_bar_chrome_keeps_maximized_state_without_host_commands() {
    let mut app = GalleryApp::new();
    let update = app.apply_message(GalleryMessage::WindowChrome(WindowChromeEvent::Action(
        WindowChromeAction::ToggleMaximize,
    )));
    assert!(
        update.window_commands.is_empty(),
        "Scene host executes title-bar window commands"
    );
    assert!(app.state().window_chrome.is_maximized());
}

#[test]
fn appearance_material_and_opacity_drive_runtime_state() {
    let mut state = GalleryState::new();
    assert_eq!(
        state.appearance.window_material(),
        WindowMaterialMode::Solid
    );
    assert!(!state.material_outcome().is_native());

    state.update(GalleryMessage::SetWindowMaterial(
        WindowMaterialMode::Translucent,
    ));
    state.update(GalleryMessage::SetBackdropOpacity(0.5));
    state.update(GalleryMessage::SetBackdropTarget(BackdropTarget::Sidebar));
    state.update(GalleryMessage::SetTitlebarFollowsSidebar(true));
    assert_eq!(
        state.appearance.window_material(),
        WindowMaterialMode::Translucent
    );
    assert!((state.appearance.backdrop_opacity() - 0.5).abs() < f32::EPSILON);

    state.update(GalleryMessage::MaterialApplied(MaterialOutcome::native(
        nana_ui::MaterialEffect::Vibrancy,
    )));
    assert!(state.material_outcome().is_native());

    // Stay on Dark (gallery default) so Reset must visibly restore Light.
    assert_eq!(state.theme_mode(), ThemeMode::Dark);
    state.update(GalleryMessage::ResetAppearance);
    assert_eq!(
        state.appearance.window_material(),
        WindowMaterialMode::Solid
    );
    assert!(
        (state.appearance.backdrop_opacity() - AppearanceSettings::DEFAULT_BACKDROP_OPACITY).abs()
            < f32::EPSILON
    );
    assert_eq!(
        state.theme_mode(),
        AppearanceSettings::RESET_THEME,
        "ResetAppearance must restore theme to Light (Lilia resetAppearanceDefaults)"
    );
}

#[test]
fn backdrop_target_drives_region_token_alphas() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::SetWindowMaterial(
        WindowMaterialMode::Translucent,
    ));
    state.update(GalleryMessage::SetBackdropOpacity(0.5));
    state.update(GalleryMessage::MaterialApplied(MaterialOutcome::native(
        nana_ui::MaterialEffect::Vibrancy,
    )));

    state.update(GalleryMessage::SetBackdropTarget(BackdropTarget::Sidebar));
    state.update(GalleryMessage::SetTitlebarFollowsSidebar(false));
    let (surface_a, background_a, titlebar_a) = state.backdrop_region_alphas();
    assert!(
        (surface_a - 0.5).abs() < f32::EPSILON,
        "Sidebar target must translucent surface even without titlebar follow"
    );
    assert!(
        (background_a - 1.0).abs() < f32::EPSILON,
        "Sidebar target keeps Primary background opaque"
    );
    assert!(
        (titlebar_a - 1.0).abs() < f32::EPSILON,
        "titlebar_follows_sidebar=false must keep titlebar opaque"
    );

    state.update(GalleryMessage::SetTitlebarFollowsSidebar(true));
    let (_, _, titlebar_a) = state.backdrop_region_alphas();
    assert!(
        (titlebar_a - 0.5).abs() < f32::EPSILON,
        "titlebar_follows_sidebar=true must translucent titlebar with sidebar"
    );

    state.update(GalleryMessage::SetBackdropTarget(BackdropTarget::Main));
    let (surface_a, background_a, titlebar_a) = state.backdrop_region_alphas();
    assert!(
        (background_a - 0.5).abs() < f32::EPSILON,
        "Main target must translucent Primary background"
    );
    assert!(
        (surface_a - 1.0).abs() < f32::EPSILON,
        "Main target keeps sidebar surface opaque"
    );
    assert!(
        (titlebar_a - 1.0).abs() < f32::EPSILON,
        "Main target keeps titlebar opaque"
    );
}

#[test]
fn split_pane_interactions_persist_the_constrained_size() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::SplitPane(SplitPaneAction::SetSize(210.0)));
    let encoded = state
        .split_pane
        .layout_json()
        .expect("split pane layout serializes");
    state.update(GalleryMessage::SplitPane(SplitPaneAction::Reset));
    assert_eq!(state.split_pane.size(), 120.0);
    state
        .split_pane
        .restore_layout_json(&encoded)
        .expect("split pane layout restores");
    assert_eq!(state.split_pane.size(), 210.0);
}

#[test]
fn dock_gallery_mutates_the_real_layout_and_emits_host_effects() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::Dock(GalleryDock::Float {
        id: "gallery.assets".into(),
        x: 20.0,
        y: 30.0,
        width: 320.0,
        height: 240.0,
    }));
    assert_eq!(state.dock.floating.len(), 1);
    assert!(!state.dock.main.contains("gallery.assets"));
    assert!(matches!(
        state.dock_events.as_slice(),
        [DockWorkspaceEvent::OpenFloating(_)]
    ));

    let surface_id = state.dock.floating[0].id.clone();
    state.update(GalleryMessage::Dock(GalleryDock::MoveFloating {
        id: surface_id,
        x: 40.0,
        y: 50.0,
        width: 400.0,
        height: 300.0,
    }));
    assert_eq!(state.dock.floating[0].x, 40.0);
    assert_eq!(state.dock.floating[0].y, 50.0);
    assert_eq!(state.dock.floating[0].width, 400.0);
    assert_eq!(state.dock.floating[0].height, 300.0);

    state.update(GalleryMessage::Dock(GalleryDock::SetLocked(true)));
    state.update(GalleryMessage::Dock(GalleryDock::Hide(
        "gallery.navigation".into(),
    )));
    assert!(state.dock_is_visible("gallery.navigation"));

    state.update(GalleryMessage::Dock(GalleryDock::SetLocked(false)));
    state.update(GalleryMessage::Dock(GalleryDock::Hide(
        "gallery.primary".into(),
    )));
    assert!(state.dock_is_visible("gallery.primary"));
    assert!(state.dock.main.contains("gallery.primary"));

    state.update(GalleryMessage::Dock(GalleryDock::Hide(
        "gallery.navigation".into(),
    )));
    assert!(!state.dock_is_visible("gallery.navigation"));
    assert!(
        state.dock.main.contains("gallery.navigation"),
        "hide must not remove the live DockWorkspace tree"
    );

    state.update(GalleryMessage::Dock(GalleryDock::Show(
        "gallery.navigation".into(),
    )));
    assert!(state.dock_is_visible("gallery.navigation"));

    state.update(GalleryMessage::Dock(GalleryDock::Reset));
    assert!(state.dock.floating.is_empty());
    assert!(state.dock.main.contains("gallery.assets"));
    assert!(state.dock_is_visible("gallery.assets"));
    assert!(state.dock_is_visible("gallery.navigation"));
}

#[test]
fn dock_float_records_a_runtime_open_window_command() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::Dock(GalleryDock::Float {
        id: "gallery.assets".into(),
        x: 20.0,
        y: 30.0,
        width: 320.0,
        height: 240.0,
    }));
    assert!(
        state
            .dock_events
            .iter()
            .any(|event| matches!(event, DockWorkspaceEvent::OpenFloating(_)))
    );
    assert!(
        state
            .dock_window_commands
            .iter()
            .any(|command| matches!(command, WindowCommand::Open { .. })),
        "Float must map through runtime_dock_window_update into an Open command"
    );

    state.update(GalleryMessage::Dock(GalleryDock::Reset));
    assert!(state.dock.floating.is_empty());
    assert!(state.dock.main.contains("gallery.assets"));
    assert!(
        state
            .dock_window_commands
            .iter()
            .any(|command| matches!(command, WindowCommand::Close(_))),
        "Reset of a floating dock must record a Close window command"
    );
}

#[test]
fn gallery_runtime_dock_pointer_resize_persists_into_workspace() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::SelectSection(GallerySection::Workspace));
    let (start, end) = state
        .gallery_runtime_dock_handle_drag()
        .expect("workspace dock handle is laid out");
    let before = first_dock_split_ratio(&state.dock.main);

    state.update(GalleryMessage::GalleryRuntime(
        RuntimeSceneInput::PointerDown {
            button: 0,
            point: start,
        },
    ));
    state.update(GalleryMessage::GalleryRuntime(
        RuntimeSceneInput::PointerMove(end),
    ));
    state.update(GalleryMessage::GalleryRuntime(
        RuntimeSceneInput::PointerUp {
            button: 0,
            point: end,
        },
    ));

    let resized = first_dock_split_ratio(&state.dock.main);
    assert!(
        (resized - before).abs() > f32::EPSILON,
        "pointer dock resize must write back into DockWorkspace.main"
    );

    state.update(GalleryMessage::ToggleCheck(state.checked));
    assert_eq!(
        first_dock_split_ratio(&state.dock.main),
        resized,
        "gallery sync must keep the persisted dock ratio"
    );
}

fn first_dock_split_ratio(node: &nana_ui::runtime::DockNode) -> f32 {
    match node {
        nana_ui::runtime::DockNode::Split { ratio, .. } => *ratio,
        _ => panic!("gallery dock main is a split"),
    }
}

#[test]
fn dock_float_backs_a_runtime_document_for_the_window() {
    let mut app = super::GalleryApp::new();
    let update = app.apply_message(GalleryMessage::Dock(GalleryDock::Float {
        id: "gallery.assets".into(),
        x: 20.0,
        y: 30.0,
        width: 320.0,
        height: 240.0,
    }));
    assert!(
        update
            .window_commands
            .iter()
            .any(|command| matches!(command, WindowCommand::Open { .. }))
    );
    let surface = app.state().dock.floating.first().expect("floated surface");
    let id = nana_ui_platform::WindowId(nana_ui::runtime::dock_surface_window_key(&surface.id));
    assert!(
        nana_ui::RuntimeProgram::document(&app, id)
            .is_some_and(|document| !document.scene().is_empty()),
        "Open must be backed by a flushed dock document"
    );

    let close = app.apply_message(GalleryMessage::Dock(GalleryDock::Reset));
    assert!(
        close
            .window_commands
            .iter()
            .any(|command| matches!(command, WindowCommand::Close(_)))
    );
    assert!(nana_ui::RuntimeProgram::document(&app, id).is_none());
}

#[test]
fn dock_float_platform_moved_persists_without_move_command() {
    let mut app = super::GalleryApp::new();
    let _ = app.apply_message(GalleryMessage::Dock(GalleryDock::Float {
        id: "gallery.assets".into(),
        x: 20.0,
        y: 30.0,
        width: 320.0,
        height: 240.0,
    }));
    let surface = app.state().dock.floating.first().expect("floated surface");
    let id = nana_ui_platform::WindowId(nana_ui::runtime::dock_surface_window_key(&surface.id));
    let commands_before = app.state().dock_window_commands.len();

    let update = app.apply_window_geometry(
        id,
        nana_ui_platform::WindowGeometry {
            physical_position: None,
            physical_size: (80, 90),
            logical_position: Some((80.0, 90.0)),
            logical_size: (360.0, 280.0),
            scale_factor: 1.0,
            maximized: false,
        },
    );
    let floating = app.state().dock.floating.first().expect("floated surface");
    assert_eq!(floating.x, 80.0);
    assert_eq!(floating.y, 90.0);
    assert_eq!(floating.width, 360.0);
    assert_eq!(floating.height, 280.0);
    assert_eq!(
        app.state().dock_window_commands.len(),
        commands_before,
        "platform Moved must not record WindowCommand::Move"
    );
    assert!(
        !update
            .window_commands
            .iter()
            .any(|command| matches!(command, WindowCommand::Move { .. })),
        "platform Moved must not echo WindowCommand::Move"
    );

    let commands_before = app.state().dock_window_commands.len();
    let _ = app.apply_window_geometry(
        id,
        nana_ui_platform::WindowGeometry {
            physical_position: None,
            physical_size: (90, 100),
            logical_position: None,
            logical_size: (400.0, 300.0),
            scale_factor: 1.0,
            maximized: false,
        },
    );
    let floating = app.state().dock.floating.first().expect("floated surface");
    assert_eq!(floating.x, 80.0);
    assert_eq!(floating.y, 90.0);
    assert_eq!(floating.width, 400.0);
    assert_eq!(floating.height, 300.0);
    assert_eq!(
        app.state().dock_window_commands.len(),
        commands_before,
        "platform Resized must not record WindowCommand::Move"
    );

    let _ = app.handle_window_event(nana_ui_platform::WindowEvent::Moved {
        id: nana_ui_platform::WindowId::PRIMARY,
        geometry: nana_ui_platform::WindowGeometry {
            physical_position: Some((12, 24)),
            physical_size: (1024, 768),
            logical_position: Some((12.0, 24.0)),
            logical_size: (1024.0, 768.0),
            scale_factor: 1.0,
            maximized: false,
        },
    });
    assert_eq!(
        app.state().window_size,
        None,
        "primary Moved must not apply WorkspaceAction::WindowResized"
    );
}
