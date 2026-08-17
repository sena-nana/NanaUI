use std::time::{Duration, Instant};

#[cfg(test)]
use crate::geometry::RESIZE_HANDLE_SIZE;
use crate::geometry::WorkspaceGeometry;
#[cfg(test)]
use crate::layout::RegionPlacement;
use crate::layout::{RegionId, WorkspaceLayout};

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RegionEdges {
    pub start: bool,
    pub end: bool,
}

#[cfg(test)]
pub(crate) fn primary_edges(
    expanded: bool,
    has_track_before: bool,
    has_track_after: bool,
) -> RegionEdges {
    RegionEdges {
        start: expanded && !has_track_before,
        end: expanded && !has_track_after,
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HandleOffset {
    pub x: f32,
    pub y: f32,
}

#[cfg(test)]
pub(crate) fn resize_handle_translation(placement: RegionPlacement) -> HandleOffset {
    let offset = RESIZE_HANDLE_SIZE / 2.0;
    match placement {
        RegionPlacement::Start | RegionPlacement::Primary => HandleOffset { x: offset, y: 0.0 },
        RegionPlacement::End => HandleOffset { x: -offset, y: 0.0 },
        RegionPlacement::Top => HandleOffset { x: 0.0, y: offset },
        RegionPlacement::Bottom => HandleOffset { x: 0.0, y: -offset },
    }
}

/// Framework-owned workspace interaction message.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceAction {
    ToggleRegion(RegionId),
    SetRegionCollapsed(RegionId, bool),
    SetRegionVisible(RegionId, bool),
    SetRegionSize(RegionId, f32),
    ResetRegionSize(RegionId),
    ResizeStart(RegionId),
    ResizeHover(Option<RegionId>),
    ResizeMove { x: f32, y: f32 },
    ResizeEnd,
    WindowResized { width: f32, height: f32 },
    WindowScaleFactorChanged(f32),
    AnimationFrame(Duration),
}

/// Owns region registrations, persisted layout, resize interaction, and host
/// viewport geometry. Application content remains outside the controller.
#[derive(Debug, Clone)]
pub struct WorkspaceController {
    model: nana_ui_core::WorkspaceModel,
    clock_origin: Instant,
}

impl Default for WorkspaceController {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceController {
    pub fn new() -> Self {
        Self::with_layout(WorkspaceLayout::default())
    }

    pub fn with_layout(layout: WorkspaceLayout) -> Self {
        Self {
            model: nana_ui_core::WorkspaceModel::with_layout(layout),
            clock_origin: Instant::now(),
        }
    }

    pub fn model(&self) -> &nana_ui_core::WorkspaceModel {
        &self.model
    }

    pub fn layout(&self) -> &WorkspaceLayout {
        self.model.layout()
    }

    pub fn layout_mut(&mut self) -> &mut WorkspaceLayout {
        self.model.layout_mut()
    }

    pub fn replace_layout(&mut self, layout: WorkspaceLayout) -> WorkspaceLayout {
        self.model.replace_layout(layout)
    }

    pub fn inline_size(&self) -> f32 {
        self.model.inline_size()
    }

    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        self.model.layout_json()
    }

    pub fn restore_layout_json(&mut self, value: &str) -> Result<(), serde_json::Error> {
        self.model.restore_layout_json(value)
    }

    pub fn geometry(
        &self,
        logical_width: f32,
        logical_height: f32,
        scale_factor: f32,
    ) -> WorkspaceGeometry {
        self.model
            .geometry(logical_width, logical_height, scale_factor)
    }

    pub fn viewport_geometry(&self) -> WorkspaceGeometry {
        self.model.viewport_geometry()
    }

    /// Applies one framework action and reports whether observable state changed.
    pub fn update(&mut self, action: WorkspaceAction) -> bool {
        let now = match action {
            WorkspaceAction::AnimationFrame(now) => now,
            _ => self.clock_origin.elapsed(),
        };
        self.update_at(action, now)
    }

    /// Deterministic backend-neutral entry used by hosted runtimes and tests.
    pub fn update_at(&mut self, action: WorkspaceAction, now: Duration) -> bool {
        let mutation = match action {
            WorkspaceAction::ToggleRegion(region) => {
                nana_ui_core::WorkspaceMutation::ToggleRegion(region)
            }
            WorkspaceAction::SetRegionCollapsed(region, collapsed) => {
                nana_ui_core::WorkspaceMutation::SetRegionCollapsed(region, collapsed)
            }
            WorkspaceAction::SetRegionVisible(region, visible) => {
                nana_ui_core::WorkspaceMutation::SetRegionVisible(region, visible)
            }
            WorkspaceAction::SetRegionSize(region, size) => {
                nana_ui_core::WorkspaceMutation::SetRegionSize(region, size)
            }
            WorkspaceAction::ResetRegionSize(region) => {
                nana_ui_core::WorkspaceMutation::ResetRegionSize(region)
            }
            WorkspaceAction::ResizeStart(region) => {
                nana_ui_core::WorkspaceMutation::ResizeStart(region)
            }
            WorkspaceAction::ResizeHover(region) => {
                nana_ui_core::WorkspaceMutation::ResizeHover(region)
            }
            WorkspaceAction::ResizeMove { x, y } => {
                nana_ui_core::WorkspaceMutation::ResizeMove { x, y }
            }
            WorkspaceAction::ResizeEnd => nana_ui_core::WorkspaceMutation::ResizeEnd,
            WorkspaceAction::WindowResized { width, height } => {
                nana_ui_core::WorkspaceMutation::SetViewport { width, height }
            }
            WorkspaceAction::WindowScaleFactorChanged(scale_factor) => {
                nana_ui_core::WorkspaceMutation::SetScaleFactor(scale_factor)
            }
            WorkspaceAction::AnimationFrame(_) => {
                nana_ui_core::WorkspaceMutation::AdvanceAnimations
            }
        };
        self.model.update(mutation, now)
    }

    #[cfg(test)]
    fn region_extent(&self, region: &RegionId) -> f32 {
        self.model.region_extent(region)
    }
}

#[cfg(test)]
#[path = "workspace_parts/tests.rs"]
mod tests;
