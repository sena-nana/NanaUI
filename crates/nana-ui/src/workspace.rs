use std::collections::{HashMap, VecDeque};

use iced::widget::canvas::{Fill, Path, Style, fill};
use iced::widget::{canvas, container, row, space, stack};
use iced::{
    Animation, Element, Length, Padding, Point, Rectangle, Renderer, Subscription, Theme, mouse,
};

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

const REGION_COLLAPSE_DURATION: iced::time::Duration = iced::time::Duration::from_millis(240);

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
    AnimationFrame(iced::time::Instant),
}

#[derive(Debug, Clone)]
struct ResizeState {
    region: RegionId,
    last_position: Option<Point>,
}

#[derive(Debug, Clone)]
struct RegionTransition {
    expansion: Animation<bool>,
    target_collapsed: bool,
    overlay: bool,
}

/// Owns region registrations, persisted layout, resize interaction, and host
/// viewport geometry. Application content remains outside the controller.
#[derive(Debug, Clone)]
pub struct WorkspaceController {
    layout: WorkspaceLayout,
    transitions: HashMap<RegionId, RegionTransition>,
    resizing: Option<ResizeState>,
    hovered_resize: Option<RegionId>,
    window_width: f32,
    window_height: f32,
    scale_factor: f32,
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
            layout,
            transitions: HashMap::new(),
            resizing: None,
            hovered_resize: None,
            window_width: 1440.0,
            window_height: 900.0,
            scale_factor: 1.0,
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
        self.window_width
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
        self.geometry(self.window_width, self.window_height, self.scale_factor)
    }

    pub fn subscription(&self) -> Subscription<WorkspaceAction> {
        let mut subscriptions = vec![iced::event::listen_with(window_event)];
        if !self.transitions.is_empty() {
            subscriptions.push(iced::window::frames().map(WorkspaceAction::AnimationFrame));
        }
        Subscription::batch(subscriptions)
    }

    /// Applies one framework action and reports whether observable state changed.
    pub fn update(&mut self, action: WorkspaceAction) -> bool {
        match action {
            WorkspaceAction::ToggleRegion(region) => {
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
            WorkspaceAction::SetRegionCollapsed(region, collapsed) => {
                self.set_region_collapsed(region, collapsed)
            }
            WorkspaceAction::SetRegionVisible(region, visible) => {
                self.transitions.remove(&region);
                self.layout.set_hidden(&region, !visible)
            }
            WorkspaceAction::SetRegionSize(region, size) => self.layout.set_size(&region, size),
            WorkspaceAction::ResetRegionSize(region) => self.layout.reset_size(&region),
            WorkspaceAction::ResizeStart(region) => {
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
                self.hovered_resize = Some(region.clone());
                self.resizing = Some(ResizeState {
                    region,
                    last_position: None,
                });
                true
            }
            WorkspaceAction::ResizeHover(region) => {
                if self.hovered_resize == region {
                    false
                } else {
                    self.hovered_resize = region;
                    true
                }
            }
            WorkspaceAction::ResizeMove { x, y } => {
                let Some(resizing) = &mut self.resizing else {
                    return false;
                };
                let Some(region) = self.layout.region(&resizing.region) else {
                    self.resizing = None;
                    return false;
                };
                let placement = region.placement_value();
                let position = Point::new(x, y);
                let changed = resizing.last_position.is_some_and(|last_position| {
                    let delta = resize_delta(placement, last_position, position);
                    self.layout.resize_by(&resizing.region, delta)
                });
                resizing.last_position = Some(position);
                changed
            }
            WorkspaceAction::ResizeEnd => {
                let changed = self.resizing.is_some() || self.hovered_resize.is_some();
                self.resizing = None;
                self.hovered_resize = None;
                changed
            }
            WorkspaceAction::WindowResized { width, height } => {
                let width = finite_non_negative(width);
                let height = finite_non_negative(height);
                let changed = self.window_width != width || self.window_height != height;
                self.window_width = width;
                self.window_height = height;
                changed
            }
            WorkspaceAction::WindowScaleFactorChanged(scale_factor) => {
                if !scale_factor.is_finite() || scale_factor <= 0.0 {
                    return false;
                }
                let changed = self.scale_factor != scale_factor;
                self.scale_factor = scale_factor;
                changed
            }
            WorkspaceAction::AnimationFrame(now) => {
                let had_transitions = !self.transitions.is_empty();
                self.transitions
                    .retain(|_, transition| transition.expansion.is_animating(now));
                had_transitions
            }
        }
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
        if current_target == collapsed {
            return false;
        }
        if !state.collapsible_value() || state.hidden_value() || state.disabled_value() {
            return false;
        }

        let now = iced::time::Instant::now();
        let overlay = self.transitions.get(&region).map_or_else(
            || state.responsive_overlay(self.window_width),
            |value| value.overlay,
        );
        if !self.layout.set_collapsed(&region, collapsed) {
            return false;
        }
        if let Some(transition) = self.transitions.get_mut(&region) {
            transition.expansion.go_mut(!collapsed, now);
            transition.target_collapsed = collapsed;
        } else {
            let mut expansion = Animation::new(!current_target)
                .duration(REGION_COLLAPSE_DURATION)
                .easing(iced::animation::Easing::EaseOutCubic);
            expansion.go_mut(!collapsed, now);
            self.transitions.insert(
                region,
                RegionTransition {
                    expansion,
                    target_collapsed: collapsed,
                    overlay,
                },
            );
        }
        true
    }

    fn region_visible(&self, state: &RegionState) -> bool {
        if self.transitions.contains_key(state.id()) {
            !state.hidden_value() && !state.responsive_collapsed(self.inline_size())
        } else {
            state.visible_at(self.inline_size())
        }
    }

    fn region_overlay(&self, state: &RegionState) -> bool {
        self.transitions.get(state.id()).map_or_else(
            || state.responsive_overlay(self.inline_size()),
            |value| value.overlay,
        )
    }

    fn region_extent(&self, region: &RegionId) -> f32 {
        self.region_extent_at(region, iced::time::Instant::now())
    }

    fn region_extent_at(&self, region: &RegionId, at: iced::time::Instant) -> f32 {
        let Some(state) = self.layout.region(region) else {
            return 0.0;
        };
        self.transitions.get(region).map_or_else(
            || state.extent(),
            |transition| transition.expansion.interpolate(0.0, state.extent(), at),
        )
    }

    fn resize_highlighted(&self, region: &RegionId) -> bool {
        self.hovered_resize.as_ref() == Some(region)
            || self
                .resizing
                .as_ref()
                .is_some_and(|state| &state.region == region)
    }
}

fn resize_delta(placement: RegionPlacement, previous: Point, current: Point) -> f32 {
    match placement {
        RegionPlacement::Start | RegionPlacement::Primary => current.x - previous.x,
        RegionPlacement::End => previous.x - current.x,
        RegionPlacement::Top => current.y - previous.y,
        RegionPlacement::Bottom => previous.y - current.y,
    }
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
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
