use super::{
    ContextAction, ContextMenuEvent, GalleryMessage, GalleryOverlay, GallerySection, GalleryState,
    SurfaceView,
};
use crate::layout::RegionId;
use crate::selection::SelectionMove;
use crate::theme::ThemeMode;
use crate::workspace::WorkspaceAction;

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
