use super::view::{RegionEdges, primary_edges};
use super::{WorkspaceAction, WorkspaceController};
use crate::layout::{RegionId, RegionRole, RegionState, WorkspaceLayout};

#[test]
fn controller_resizes_regions_in_their_visual_direction() {
    let mut controller = WorkspaceController::new();
    let resources = size(&controller, &RegionId::Resources);
    let inspector = size(&controller, &RegionId::Inspector);
    let diagnostics = size(&controller, &RegionId::Diagnostics);

    resize(
        &mut controller,
        RegionId::Resources,
        (10.0, 0.0),
        (34.0, 0.0),
    );
    resize(
        &mut controller,
        RegionId::Inspector,
        (100.0, 0.0),
        (76.0, 0.0),
    );
    resize(
        &mut controller,
        RegionId::Diagnostics,
        (0.0, 100.0),
        (0.0, 76.0),
    );

    assert_eq!(size(&controller, &RegionId::Resources), resources + 24.0);
    assert_eq!(size(&controller, &RegionId::Inspector), inspector + 24.0);
    assert_eq!(
        size(&controller, &RegionId::Diagnostics),
        diagnostics + 24.0
    );
}

#[test]
fn controller_resizes_application_defined_regions() {
    let pull_requests = RegionId::custom("pull-requests");
    let layout = WorkspaceLayout::new([
        RegionState::new(pull_requests.clone(), RegionRole::SectionNavigation)
            .size(230.0)
            .resizable(true),
        RegionState::new(RegionId::Primary, RegionRole::Primary),
    ])
    .expect("dynamic layout");
    let mut controller = WorkspaceController::with_layout(layout);

    resize(
        &mut controller,
        pull_requests.clone(),
        (0.0, 0.0),
        (30.0, 0.0),
    );
    assert_eq!(size(&controller, &pull_requests), 260.0);
}

#[test]
fn controller_rejects_non_resizable_and_collapsed_regions() {
    let mut controller = WorkspaceController::new();
    assert!(!controller.update(WorkspaceAction::ResizeStart(RegionId::GlobalNavigation)));

    assert!(controller.update(WorkspaceAction::ToggleRegion(RegionId::Resources)));
    assert!(!controller.update(WorkspaceAction::ResizeStart(RegionId::Resources)));
}

#[test]
fn controller_restores_a_resized_region_to_its_default() {
    let mut controller = WorkspaceController::new();
    resize(
        &mut controller,
        RegionId::Resources,
        (10.0, 0.0),
        (50.0, 0.0),
    );
    assert!(controller.update(WorkspaceAction::ResetRegionSize(RegionId::Resources)));
    assert_eq!(size(&controller, &RegionId::Resources), 260.0);
}

#[test]
fn controller_applies_deterministic_region_state() {
    let mut controller = WorkspaceController::new();
    assert!(controller.update(WorkspaceAction::SetRegionCollapsed(
        RegionId::Resources,
        true,
    )));
    assert!(!controller.update(WorkspaceAction::SetRegionCollapsed(
        RegionId::Resources,
        true,
    )));
    assert!(controller.update(WorkspaceAction::SetRegionSize(RegionId::Inspector, 320.0,)));
    assert_eq!(size(&controller, &RegionId::Inspector), 320.0);
}

#[test]
fn controller_animates_region_extent_and_commits_the_target_immediately() {
    let mut controller = WorkspaceController::new();
    let started = iced::time::Instant::now();

    assert!(controller.update(WorkspaceAction::SetRegionCollapsed(
        RegionId::Resources,
        true,
    )));
    assert!(
        controller
            .layout()
            .region(&RegionId::Resources)
            .expect("resources")
            .collapsed_value()
    );

    let middle = controller.region_extent_at(
        &RegionId::Resources,
        started + iced::time::Duration::from_millis(120),
    );
    assert!(middle > 0.0 && middle < 260.0);

    let finished = started + iced::time::Duration::from_millis(300);
    assert_eq!(
        controller.region_extent_at(&RegionId::Resources, finished),
        0.0
    );
    assert!(controller.update(WorkspaceAction::AnimationFrame(finished)));
    assert!(!controller.transitions.contains_key(&RegionId::Resources));
}

#[test]
fn controller_reverses_an_active_collapse_without_losing_region_state() {
    let mut controller = WorkspaceController::new();
    assert!(controller.update(WorkspaceAction::SetRegionCollapsed(
        RegionId::Resources,
        true,
    )));
    assert!(controller.update(WorkspaceAction::SetRegionCollapsed(
        RegionId::Resources,
        false,
    )));
    assert!(
        !controller
            .layout()
            .region(&RegionId::Resources)
            .expect("resources")
            .collapsed_value()
    );
    assert!(
        !controller
            .transitions
            .get(&RegionId::Resources)
            .expect("transition")
            .target_collapsed
    );
}

#[test]
fn controller_owns_serialized_layout_and_viewport_geometry() {
    let mut controller = WorkspaceController::new();
    controller.update(WorkspaceAction::ToggleRegion(RegionId::Inspector));
    controller.update(WorkspaceAction::WindowResized {
        width: 1000.0,
        height: 700.0,
    });
    controller.update(WorkspaceAction::WindowScaleFactorChanged(1.5));

    let encoded = controller.layout_json().expect("layout serializes");
    let mut restored = WorkspaceController::new();
    restored
        .restore_layout_json(&encoded)
        .expect("layout restores");

    assert_eq!(restored.layout(), controller.layout());
    assert_eq!(controller.viewport_geometry().physical_size, (1500, 1050));
}

#[test]
fn primary_corners_follow_the_first_and_last_expanded_middle_tracks() {
    assert_eq!(
        primary_edges(true, true, true),
        RegionEdges {
            start: false,
            end: false,
        }
    );
    assert_eq!(
        primary_edges(true, false, true),
        RegionEdges {
            start: true,
            end: false,
        }
    );
    assert_eq!(
        primary_edges(true, true, false),
        RegionEdges {
            start: false,
            end: true,
        }
    );
    assert_eq!(
        primary_edges(true, false, false),
        RegionEdges {
            start: true,
            end: true,
        }
    );
    assert_eq!(primary_edges(false, false, false), RegionEdges::default());
}

fn resize(
    controller: &mut WorkspaceController,
    region: RegionId,
    start: (f32, f32),
    end: (f32, f32),
) {
    assert!(controller.update(WorkspaceAction::ResizeStart(region)));
    controller.update(WorkspaceAction::ResizeMove {
        x: start.0,
        y: start.1,
    });
    assert!(controller.update(WorkspaceAction::ResizeMove { x: end.0, y: end.1 }));
    assert!(controller.update(WorkspaceAction::ResizeEnd));
}

fn size(controller: &WorkspaceController, region: &RegionId) -> f32 {
    controller
        .layout()
        .region(region)
        .and_then(RegionState::size_value)
        .expect("fixed region size")
}
