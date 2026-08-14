use std::collections::VecDeque;
use std::time::{Duration, Instant};

use iced::widget::canvas::{Fill, Path, Style, fill};
use iced::widget::{canvas, container, row, space, stack};
use iced::{Element, Length, Padding, Rectangle, Renderer, Subscription, Theme, mouse};

use crate::geometry::{RESIZE_HANDLE_SIZE, WorkspaceGeometry};
use crate::layout::{
    RegionId, RegionPlacement, RegionRole, RegionScope, RegionState, WorkspaceLayout,
};
use crate::theme::{Colors, ThemeTokens};
use crate::widgets::{primary_region_radius, primary_region_style, workspace_region_style};

#[path = "workspace_parts/regions.rs"]
mod regions;
#[path = "workspace_parts/view.rs"]
mod view;

pub use regions::{WorkspaceRegion, WorkspaceRegions, WorkspaceSlots};
pub use view::workspace_view;

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

    pub fn subscription(&self) -> Subscription<WorkspaceAction> {
        let mut subscriptions = vec![iced::event::listen_with(window_event)];
        if self.model.has_active_transitions() {
            let origin = self.clock_origin;
            subscriptions.push(iced::window::frames().with(origin).map(|(origin, now)| {
                WorkspaceAction::AnimationFrame(now.saturating_duration_since(origin))
            }));
        }
        Subscription::batch(subscriptions)
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

    fn region_visible(&self, state: &RegionState) -> bool {
        self.model.region_visible(state)
    }

    fn region_overlay(&self, state: &RegionState) -> bool {
        self.model.region_overlay(state)
    }

    fn region_extent(&self, region: &RegionId) -> f32 {
        self.model.region_extent(region)
    }

    fn region_transitioning(&self, region: &RegionId) -> bool {
        self.model.region_transitioning(region)
    }

    fn resize_highlighted(&self, region: &RegionId) -> bool {
        self.model.resize_highlighted(region)
    }
}

fn window_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<WorkspaceAction> {
    match event {
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(WorkspaceAction::WindowResized {
                width: size.width,
                height: size.height,
            })
        }
        iced::Event::Window(iced::window::Event::Rescaled(scale_factor)) => {
            Some(WorkspaceAction::WindowScaleFactorChanged(scale_factor))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "workspace_parts/tests.rs"]
mod tests;
