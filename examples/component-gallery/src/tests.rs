use super::{
    ContextAction, ContextMenuEvent, GalleryMessage, GalleryOverlay, GallerySection, GalleryState,
    SurfaceView,
};
use nana_ui::{
    AppearanceSettings, BackdropTarget, DockAction, DockHostEffect, DockId, MaterialOutcome,
    RegionId, SelectionMove, SplitPaneAction, ThemeMode, WindowMaterialMode, WorkspaceAction,
};

#[test]
fn gallery_interactions_update_real_state() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::PrimaryAction);
    state.update(GalleryMessage::ToggleLoading);
    state.update(GalleryMessage::InputChanged("Field".to_owned()));
    state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
    state.update(GalleryMessage::ToggleContextMenu);
    state.update(GalleryMessage::ContextMenu(ContextMenuEvent::Select(
        ContextAction::Rename,
    )));

    assert_eq!(state.primary_clicks, 1);
    assert!(state.loading);
    assert_eq!(state.input, "Field");
    assert_eq!(state.section, GallerySection::Feedback);
    assert!(!state.overlay.is_open());
    assert_eq!(state.context_action, Some(ContextAction::Rename));
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

    state.update(GalleryMessage::ContextMenu(ContextMenuEvent::Select(
        ContextAction::Remove,
    )));
    assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
    assert_eq!(
        state.menu_confirmation.pending(),
        Some(&ContextAction::Remove)
    );
    assert_eq!(state.selected_item, 2);

    state.update(GalleryMessage::ContextMenu(ContextMenuEvent::Select(
        ContextAction::Remove,
    )));
    assert!(!state.overlay.is_open());
    assert_eq!(state.context_action, Some(ContextAction::Remove));
    assert_eq!(state.selected_item, 0);
}

#[test]
fn dialog_confirmation_executes_and_closes_the_overlay() {
    let mut state = GalleryState::new();
    state.update(GalleryMessage::ToggleDialog);
    assert!(state.overlay.contains(&GalleryOverlay::Dialog));

    state.update(GalleryMessage::ConfirmDialog);
    assert!(!state.overlay.is_open());
    assert_eq!(state.confirmed_actions, 1);
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
    state.update(GalleryMessage::ContextMenu(ContextMenuEvent::Search(
        "重命名".to_owned(),
    )));
    assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
    assert_eq!(state.context_query, "重命名");
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
    assert!(state.material_outcome().status_label().contains("Vibrancy"));

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
    state.update(GalleryMessage::Dock(DockAction::Float {
        id: DockId::from("gallery.sources"),
        bounds: nana_ui::DockBounds::new(20.0, 30.0, 320.0, 240.0),
        monitor: None,
    }));
    assert_eq!(state.dock.layout().floating.len(), 1);
    assert!(matches!(
        state.dock_effects.as_slice(),
        [DockHostEffect::OpenFloating(_)]
    ));

    state.update(GalleryMessage::Dock(DockAction::SetLocked(true)));
    state.update(GalleryMessage::Dock(DockAction::Hide(DockId::from(
        "gallery.scenes",
    ))));
    assert!(state.dock.is_visible(&DockId::from("gallery.scenes")));

    state.update(GalleryMessage::Dock(DockAction::SetLocked(false)));
    state.update(GalleryMessage::Dock(DockAction::Reset));
    assert!(state.dock.layout().floating.is_empty());
    assert!(state.dock.is_visible(&DockId::from("gallery.sources")));
}
