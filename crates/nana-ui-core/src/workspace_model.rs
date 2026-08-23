//! Backend-neutral workspace interaction and transition authority.

use std::collections::HashMap;
use std::time::Duration;

use crate::{RegionId, RegionPlacement, RegionState, WorkspaceGeometry, WorkspaceLayout};

pub const WORKSPACE_REGION_TRANSITION_DURATION: Duration = Duration::from_millis(240);

#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceMutation {
    ToggleRegion(RegionId),
    SetRegionCollapsed(RegionId, bool),
    SetRegionVisible(RegionId, bool),
    SetRegionSize(RegionId, f32),
    ResetRegionSize(RegionId),
    ResizeStart(RegionId),
    ResizeHover(Option<RegionId>),
    ResizeMove { x: f32, y: f32 },
    ResizeEnd,
    SetViewport { width: f32, height: f32 },
    SetScaleFactor(f32),
    AdvanceAnimations,
}

#[derive(Debug, Clone, PartialEq)]
struct ResizeState {
    region: RegionId,
    axis: ResizeAxis,
    direction: f32,
    start_position: Option<f32>,
    start_extent: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ResizeAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq)]
struct RegionTransition {
    started_at: Duration,
    from_extent: f32,
    to_extent: f32,
    target_collapsed: bool,
    overlay: bool,
}

impl RegionTransition {
    fn extent_at(&self, now: Duration) -> f32 {
        let elapsed = now.saturating_sub(self.started_at);
        let linear = (elapsed.as_secs_f32() / WORKSPACE_REGION_TRANSITION_DURATION.as_secs_f32())
            .clamp(0.0, 1.0);
        let progress = 1.0 - (1.0 - linear).powi(3);
        self.from_extent + (self.to_extent - self.from_extent) * progress
    }

    fn finished_at(&self, now: Duration) -> bool {
        now.saturating_sub(self.started_at) >= WORKSPACE_REGION_TRANSITION_DURATION
    }
}

/// Owns persisted region layout, resize interaction, viewport facts and
/// collapse transitions without depending on a window or renderer backend.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceModel {
    layout: WorkspaceLayout,
    transitions: HashMap<RegionId, RegionTransition>,
    resizing: Option<ResizeState>,
    hovered_resize: Option<RegionId>,
    viewport_width: f32,
    viewport_height: f32,
    scale_factor: f32,
    now: Duration,
}

impl Default for WorkspaceModel {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceModel {
    pub fn new() -> Self {
        Self::with_layout(WorkspaceLayout::default())
    }

    pub fn with_layout(layout: WorkspaceLayout) -> Self {
        Self {
            layout,
            transitions: HashMap::new(),
            resizing: None,
            hovered_resize: None,
            viewport_width: 1440.0,
            viewport_height: 900.0,
            scale_factor: 1.0,
            now: Duration::ZERO,
        }
    }

    pub fn layout(&self) -> &WorkspaceLayout {
        &self.layout
    }

    pub fn layout_mut(&mut self) -> &mut WorkspaceLayout {
        self.transitions.clear();
        &mut self.layout
    }

    pub fn replace_layout(&mut self, layout: WorkspaceLayout) -> WorkspaceLayout {
        self.resizing = None;
        self.hovered_resize = None;
        self.transitions.clear();
        std::mem::replace(&mut self.layout, layout)
    }

    pub fn inline_size(&self) -> f32 {
        self.viewport_width
    }

    pub fn has_active_transitions(&self) -> bool {
        !self.transitions.is_empty()
    }

    pub fn region_transitioning(&self, region: &RegionId) -> bool {
        self.transitions.contains_key(region)
    }

    pub fn geometry(
        &self,
        logical_width: f32,
        logical_height: f32,
        scale_factor: f32,
    ) -> WorkspaceGeometry {
        if self.transitions.is_empty() {
            WorkspaceGeometry::new(&self.layout, logical_width, logical_height, scale_factor)
        } else {
            let layout = self.layout.with_transient_extents(
                self.transitions
                    .keys()
                    .map(|id| (id.clone(), self.region_extent(id))),
            );
            WorkspaceGeometry::new(&layout, logical_width, logical_height, scale_factor)
        }
    }

    pub fn viewport_geometry(&self) -> WorkspaceGeometry {
        self.geometry(self.viewport_width, self.viewport_height, self.scale_factor)
    }

    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        self.layout.to_json()
    }

    pub fn restore_layout_json(&mut self, value: &str) -> Result<(), serde_json::Error> {
        self.layout.restore_json(value)?;
        self.resizing = None;
        self.hovered_resize = None;
        self.transitions.clear();
        Ok(())
    }

    pub fn update(&mut self, mutation: WorkspaceMutation, now: Duration) -> bool {
        self.now = self.now.max(now);
        match mutation {
            WorkspaceMutation::ToggleRegion(region) => {
                let Some(state) = self.layout.region(&region) else {
                    return false;
                };
                let collapsed = self
                    .transitions
                    .get(&region)
                    .map_or(state.collapsed_value(), |transition| {
                        transition.target_collapsed
                    });
                self.set_region_collapsed(region, !collapsed)
            }
            WorkspaceMutation::SetRegionCollapsed(region, collapsed) => {
                self.set_region_collapsed(region, collapsed)
            }
            WorkspaceMutation::SetRegionVisible(region, visible) => {
                let transition_removed = self.transitions.remove(&region).is_some();
                self.layout.set_hidden(&region, !visible) || transition_removed
            }
            WorkspaceMutation::SetRegionSize(region, size) => {
                let transition_removed = self.transitions.remove(&region).is_some();
                self.layout.set_size(&region, size) || transition_removed
            }
            WorkspaceMutation::ResetRegionSize(region) => {
                let transition_removed = self.transitions.remove(&region).is_some();
                self.layout.reset_size(&region) || transition_removed
            }
            WorkspaceMutation::ResizeStart(region) => self.start_resize(region),
            WorkspaceMutation::ResizeHover(region) => {
                if self.hovered_resize == region {
                    false
                } else {
                    self.hovered_resize = region;
                    true
                }
            }
            WorkspaceMutation::ResizeMove { x, y } => self.resize_move(x, y),
            WorkspaceMutation::ResizeEnd => {
                let changed = self.resizing.is_some() || self.hovered_resize.is_some();
                self.resizing = None;
                self.hovered_resize = None;
                changed
            }
            WorkspaceMutation::SetViewport { width, height } => {
                let width = finite_non_negative(width);
                let height = finite_non_negative(height);
                let changed = self.viewport_width != width || self.viewport_height != height;
                self.viewport_width = width;
                self.viewport_height = height;
                changed
            }
            WorkspaceMutation::SetScaleFactor(scale_factor) => {
                if !scale_factor.is_finite() || scale_factor <= 0.0 {
                    return false;
                }
                let changed = self.scale_factor != scale_factor;
                self.scale_factor = scale_factor;
                changed
            }
            WorkspaceMutation::AdvanceAnimations => {
                let had_transitions = !self.transitions.is_empty();
                self.transitions
                    .retain(|_, transition| !transition.finished_at(self.now));
                had_transitions
            }
        }
    }

    pub fn region_visible(&self, state: &RegionState) -> bool {
        if self.transitions.contains_key(state.id()) {
            !state.hidden_value() && !state.responsive_collapsed(self.inline_size())
        } else {
            state.visible_at(self.inline_size())
        }
    }

    pub fn region_overlay(&self, state: &RegionState) -> bool {
        self.transitions.get(state.id()).map_or_else(
            || state.responsive_overlay(self.inline_size()),
            |transition| transition.overlay,
        )
    }

    pub fn region_extent(&self, region: &RegionId) -> f32 {
        let Some(state) = self.layout.region(region) else {
            return 0.0;
        };
        self.transitions.get(region).map_or_else(
            || {
                if state.collapsed_value() {
                    0.0
                } else {
                    state.extent()
                }
            },
            |transition| transition.extent_at(self.now),
        )
    }

    pub fn resize_highlighted(&self, region: &RegionId) -> bool {
        self.hovered_resize.as_ref() == Some(region)
            || self
                .resizing
                .as_ref()
                .is_some_and(|state| &state.region == region)
    }

    pub fn is_resizing(&self) -> bool {
        self.resizing.is_some()
    }

    fn set_region_collapsed(&mut self, region: RegionId, collapsed: bool) -> bool {
        let Some(state) = self.layout.region(&region) else {
            return false;
        };
        let current_target = self
            .transitions
            .get(&region)
            .map_or(state.collapsed_value(), |transition| {
                transition.target_collapsed
            });
        if current_target == collapsed
            || !state.collapsible_value()
            || state.hidden_value()
            || state.disabled_value()
        {
            return false;
        }
        let from_extent = self.region_extent(&region);
        let expanded_extent = state.extent();
        let overlay = self.transitions.get(&region).map_or_else(
            || state.responsive_overlay(self.viewport_width),
            |value| value.overlay,
        );
        if !self.layout.set_collapsed(&region, collapsed) {
            return false;
        }
        self.transitions.insert(
            region,
            RegionTransition {
                started_at: self.now,
                from_extent,
                to_extent: if collapsed { 0.0 } else { expanded_extent },
                target_collapsed: collapsed,
                overlay,
            },
        );
        true
    }

    fn start_resize(&mut self, region: RegionId) -> bool {
        let Some(state) = self.layout.region(&region) else {
            return false;
        };
        if !state.resizable_value()
            || state.disabled_value()
            || !state.requested_visible()
            || state.fill_priority_value() > 0
            || self.transitions.contains_key(&region)
        {
            return false;
        }
        let (axis, direction) = match state.placement_value() {
            RegionPlacement::Start | RegionPlacement::Primary => (ResizeAxis::Horizontal, 1.0),
            RegionPlacement::End => (ResizeAxis::Horizontal, -1.0),
            RegionPlacement::Top => (ResizeAxis::Vertical, 1.0),
            RegionPlacement::Bottom => (ResizeAxis::Vertical, -1.0),
        };
        self.hovered_resize = Some(region.clone());
        self.resizing = Some(ResizeState {
            region,
            axis,
            direction,
            start_position: None,
            start_extent: state.extent(),
        });
        true
    }

    fn resize_move(&mut self, x: f32, y: f32) -> bool {
        let Some(resizing) = &mut self.resizing else {
            return false;
        };
        if self.layout.region(&resizing.region).is_none() {
            self.resizing = None;
            return false;
        }
        let position = match resizing.axis {
            ResizeAxis::Horizontal => x,
            ResizeAxis::Vertical => y,
        };
        if !position.is_finite() {
            return false;
        }
        let Some(start_position) = resizing.start_position else {
            resizing.start_position = Some(position);
            return false;
        };
        let extent = resizing.start_extent + (position - start_position) * resizing.direction;
        self.layout.set_size(&resizing.region, extent)
    }
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_is_deterministic_reversible_and_idle_after_deadline() {
        let mut model = WorkspaceModel::new();
        assert!(model.update(
            WorkspaceMutation::SetRegionCollapsed(RegionId::Resources, true),
            Duration::from_millis(100),
        ));
        assert!(model.region_extent(&RegionId::Resources) > 0.0);
        assert!(model.update(
            WorkspaceMutation::AdvanceAnimations,
            Duration::from_millis(220),
        ));
        let middle = model.region_extent(&RegionId::Resources);
        assert!(middle > 0.0 && middle < 260.0);
        assert!(model.update(
            WorkspaceMutation::SetRegionCollapsed(RegionId::Resources, false),
            Duration::from_millis(220),
        ));
        assert_eq!(model.region_extent(&RegionId::Resources), middle);
        assert!(model.update(
            WorkspaceMutation::AdvanceAnimations,
            Duration::from_millis(500),
        ));
        assert!(!model.has_active_transitions());
        assert_eq!(model.region_extent(&RegionId::Resources), 260.0);
        assert!(!model.update(
            WorkspaceMutation::AdvanceAnimations,
            Duration::from_millis(600),
        ));
    }

    #[test]
    fn resize_uses_absolute_pointer_delta_without_clamp_drift() {
        let mut model = WorkspaceModel::new();
        assert!(model.update(
            WorkspaceMutation::ResizeStart(RegionId::Resources),
            Duration::ZERO,
        ));
        assert!(!model.update(
            WorkspaceMutation::ResizeMove { x: 100.0, y: 0.0 },
            Duration::ZERO,
        ));
        assert!(model.update(
            WorkspaceMutation::ResizeMove {
                x: 10_000.0,
                y: 0.0
            },
            Duration::ZERO,
        ));
        assert!(model.update(
            WorkspaceMutation::ResizeMove { x: 120.0, y: 0.0 },
            Duration::ZERO,
        ));
        assert_eq!(
            model
                .layout()
                .region(&RegionId::Resources)
                .unwrap()
                .extent(),
            280.0
        );
    }

    #[test]
    fn explicit_size_cancels_transition_and_reports_the_visual_change() {
        let mut model = WorkspaceModel::new();
        assert!(model.update(
            WorkspaceMutation::SetRegionCollapsed(RegionId::Resources, true),
            Duration::ZERO,
        ));
        assert!(model.update(
            WorkspaceMutation::AdvanceAnimations,
            Duration::from_millis(120),
        ));
        assert!(model.region_extent(&RegionId::Resources) < 260.0);

        assert!(model.update(
            WorkspaceMutation::SetRegionSize(RegionId::Resources, 260.0),
            Duration::from_millis(120),
        ));
        assert!(!model.has_active_transitions());
        assert_eq!(model.region_extent(&RegionId::Resources), 0.0);
    }
}
