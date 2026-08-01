use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::widget::{button, column, container, row, space, stack, text};
use iced::{Alignment, Element, Event, Length, Point, Rectangle, Size, Subscription};
use serde::{Deserialize, Serialize};

use crate::drag_handle::DragHandle;
use crate::resize_drag::{ResizeAxis, ResizeDrag};
use crate::theme::{ThemeTokens, ui_font};
use crate::widgets::{ButtonKind, button_style};
#[cfg(feature = "hosted")]
use crate::{
    HostedProgramUpdate, HostedWindowCommand, HostedWindowEvent, HostedWindowId,
    HostedWindowSettings,
};

const DOCK_LAYOUT_VERSION: u8 = 1;
const DIVIDER_HIT_SIZE: f32 = 8.0;
const TITLE_BAR_HEIGHT: f32 = 28.0;
const MIN_SPLIT_RATIO: f32 = 0.05;
const MAX_SPLIT_RATIO: f32 = 0.95;

/// Stable application-owned identity for a dock item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DockId(String);

impl DockId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for DockId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// Host window/surface identity. NanaUI never creates the matching window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DockSurfaceId(pub u64);

#[cfg(feature = "hosted")]
impl From<DockSurfaceId> for HostedWindowId {
    fn from(value: DockSurfaceId) -> Self {
        Self(value.0)
    }
}

#[cfg(feature = "hosted")]
impl From<HostedWindowId> for DockSurfaceId {
    fn from(value: HostedWindowId) -> Self {
        Self(value.0)
    }
}

/// Direction of a dock split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockAxis {
    Horizontal,
    Vertical,
}

/// A recursive dock tree. Ratios describe the first child's share of available space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DockNode {
    Item {
        id: DockId,
    },
    Tabs {
        tabs: Vec<DockId>,
        active: DockId,
    },
    Split {
        axis: DockAxis,
        ratio: f32,
        first: Box<DockNode>,
        second: Box<DockNode>,
    },
}

impl DockNode {
    pub fn item(id: impl Into<DockId>) -> Self {
        Self::Item { id: id.into() }
    }

    pub fn tabs(tabs: impl IntoIterator<Item = DockId>, active: impl Into<DockId>) -> Self {
        Self::Tabs {
            tabs: tabs.into_iter().collect(),
            active: active.into(),
        }
    }

    pub fn split(axis: DockAxis, ratio: f32, first: DockNode, second: DockNode) -> Self {
        Self::Split {
            axis,
            ratio: clamp_ratio(ratio),
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn ids(&self, output: &mut Vec<DockId>) {
        match self {
            Self::Item { id } => output.push(id.clone()),
            Self::Tabs { tabs, .. } => output.extend(tabs.iter().cloned()),
            Self::Split { first, second, .. } => {
                first.ids(output);
                second.ids(output);
            }
        }
    }

    fn contains(&self, needle: &DockId) -> bool {
        match self {
            Self::Item { id } => id == needle,
            Self::Tabs { tabs, .. } => tabs.contains(needle),
            Self::Split { first, second, .. } => first.contains(needle) || second.contains(needle),
        }
    }
}

/// Static registration for one application dock.
#[derive(Debug, Clone, PartialEq)]
pub struct DockItemSpec {
    pub id: DockId,
    pub title: String,
    pub minimum_width: f32,
    pub minimum_height: f32,
    pub maximum_width: Option<f32>,
    pub maximum_height: Option<f32>,
    pub closeable: bool,
    pub floatable: bool,
}

impl DockItemSpec {
    pub fn new(id: impl Into<DockId>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            minimum_width: 96.0,
            minimum_height: 64.0,
            maximum_width: None,
            maximum_height: None,
            closeable: true,
            floatable: true,
        }
    }

    pub fn limits(mut self, minimum_width: f32, minimum_height: f32) -> Self {
        self.minimum_width = finite_positive(minimum_width, 96.0);
        self.minimum_height = finite_positive(minimum_height, 64.0);
        self
    }

    pub fn maximum(mut self, width: Option<f32>, height: Option<f32>) -> Self {
        self.maximum_width = width.filter(|value| value.is_finite() && *value > 0.0);
        self.maximum_height = height.filter(|value| value.is_finite() && *value > 0.0);
        self
    }

    pub fn closeable(mut self, closeable: bool) -> Self {
        self.closeable = closeable;
        self
    }

    pub fn floatable(mut self, floatable: bool) -> Self {
        self.floatable = floatable;
        self
    }
}

/// Logical floating-window bounds saved independently of a physical monitor scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DockBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl DockBounds {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Keeps a restored floating dock visible inside the selected monitor work area.
    pub fn clamped_to(self, work_area: Self) -> Self {
        let minimum_width = 160.0_f32.min(work_area.width.max(1.0));
        let minimum_height = 120.0_f32.min(work_area.height.max(1.0));
        let width = finite_positive(self.width, minimum_width)
            .clamp(minimum_width, work_area.width.max(minimum_width));
        let height = finite_positive(self.height, minimum_height)
            .clamp(minimum_height, work_area.height.max(minimum_height));
        let x =
            finite(self.x, work_area.x).clamp(work_area.x, work_area.x + work_area.width - width);
        let y =
            finite(self.y, work_area.y).clamp(work_area.y, work_area.y + work_area.height - height);
        Self::new(x, y, width, height)
    }
}

/// One host-owned floating window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloatingDock {
    pub surface: DockSurfaceId,
    pub root: DockNode,
    pub bounds: DockBounds,
    pub monitor: Option<String>,
}

/// Versioned persisted dock state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockLayout {
    pub version: u8,
    pub main: DockNode,
    #[serde(default)]
    pub floating: Vec<FloatingDock>,
    #[serde(default)]
    pub hidden: Vec<DockId>,
    #[serde(default)]
    pub locked: bool,
}

impl DockLayout {
    pub fn new(main: DockNode) -> Self {
        Self {
            version: DOCK_LAYOUT_VERSION,
            main,
            floating: Vec::new(),
            hidden: Vec::new(),
            locked: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockDropZone {
    Left,
    Right,
    Top,
    Bottom,
    Tab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockDropTarget {
    pub surface: DockSurfaceId,
    pub id: DockId,
    pub zone: DockDropZone,
}

/// All deterministic dock state transitions. Hosts may also drive these from native hit testing.
#[derive(Debug, Clone, PartialEq)]
pub enum DockAction {
    ActivateTab(DockId),
    ReorderTab {
        id: DockId,
        before: Option<DockId>,
    },
    ResizeStart {
        surface: DockSurfaceId,
        path: Vec<usize>,
    },
    ResizeMove(Point),
    ResizeEnd,
    ResizeSplit {
        surface: DockSurfaceId,
        path: Vec<usize>,
        ratio: f32,
    },
    AdjustSplit {
        surface: DockSurfaceId,
        path: Vec<usize>,
        steps: f32,
    },
    KeyboardAdjust(f32),
    BlurSplit,
    ResetSplit {
        surface: DockSurfaceId,
        path: Vec<usize>,
    },
    SurfaceResized {
        surface: DockSurfaceId,
        width: f32,
        height: f32,
    },
    SurfaceGeometry {
        surface: DockSurfaceId,
        bounds: DockBounds,
    },
    DragStart(DockId),
    DragMove(Point),
    DragEnd,
    CancelDrag,
    Hover(bool),
    Hide(DockId),
    Show(DockId),
    Float {
        id: DockId,
        bounds: DockBounds,
        monitor: Option<String>,
    },
    Dock {
        id: DockId,
        target: DockDropTarget,
    },
    Focus(DockId),
    CloseSurface(DockSurfaceId),
    SetLocked(bool),
    Reset,
}

/// Side effects the application must execute against host-owned windows.
#[derive(Debug, Clone, PartialEq)]
pub enum DockHostEffect {
    OpenFloating(FloatingDock),
    CloseFloating(DockSurfaceId),
    FocusFloating(DockSurfaceId),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DockUpdate {
    pub changed: bool,
    pub effects: Vec<DockHostEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockError {
    UnsupportedVersion(u8),
    DuplicateRegistration(DockId),
    DuplicateDock(DockId),
    MissingCenter(DockId),
    InvalidCenter(DockId),
    InvalidTabs,
    InvalidSplit,
    UnknownDock(DockId),
    InvalidJson(String),
}

impl std::fmt::Display for DockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for DockError {}

#[derive(Debug, Clone)]
struct ActiveResize {
    surface: DockSurfaceId,
    path: Vec<usize>,
    drag: ResizeDrag,
}

#[derive(Debug, Clone)]
struct ActiveDrag {
    id: DockId,
    start: Option<Point>,
    position: Option<Point>,
    moved: bool,
    target: Option<DockDropTarget>,
}

/// Owns a validated dock layout without owning native windows or GPU resources.
#[derive(Debug, Clone)]
pub struct DockController {
    center: DockId,
    specs: BTreeMap<DockId, DockItemSpec>,
    default_layout: DockLayout,
    layout: DockLayout,
    next_surface: u64,
    surface_bounds: BTreeMap<DockSurfaceId, DockBounds>,
    active_resize: Option<ActiveResize>,
    focused_split: Option<(DockSurfaceId, Vec<usize>, DockAxis)>,
    active_drag: Option<ActiveDrag>,
}

impl DockController {
    pub fn new(
        center: impl Into<DockId>,
        specs: impl IntoIterator<Item = DockItemSpec>,
        default_layout: DockLayout,
    ) -> Result<Self, DockError> {
        let center = center.into();
        let mut registry = BTreeMap::new();
        for spec in specs {
            let id = spec.id.clone();
            if registry.insert(id.clone(), spec).is_some() {
                return Err(DockError::DuplicateRegistration(id));
            }
        }
        let center_spec = registry
            .get_mut(&center)
            .ok_or_else(|| DockError::UnknownDock(center.clone()))?;
        center_spec.closeable = false;
        center_spec.floatable = false;
        validate_layout(&default_layout, &registry, &center)?;
        let next_surface = default_layout
            .floating
            .iter()
            .map(|dock| dock.surface.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Ok(Self {
            center,
            specs: registry,
            default_layout: default_layout.clone(),
            layout: default_layout,
            next_surface,
            surface_bounds: BTreeMap::from([(
                DockSurfaceId(0),
                DockBounds::new(0.0, 0.0, 1280.0, 800.0),
            )]),
            active_resize: None,
            focused_split: None,
            active_drag: None,
        })
    }

    pub fn layout(&self) -> &DockLayout {
        &self.layout
    }

    pub fn item(&self, id: &DockId) -> Option<&DockItemSpec> {
        self.specs.get(id)
    }

    pub fn is_visible(&self, id: &DockId) -> bool {
        !self.layout.hidden.contains(id)
            && (self.layout.main.contains(id)
                || self
                    .layout
                    .floating
                    .iter()
                    .any(|dock| dock.root.contains(id)))
    }

    pub fn is_dragging(&self) -> bool {
        self.active_drag.is_some()
    }

    pub fn drop_target(&self) -> Option<&DockDropTarget> {
        self.active_drag
            .as_ref()
            .and_then(|drag| drag.target.as_ref())
    }

    /// Provides arrow-key adjustment after a divider has been clicked or dragged.
    pub fn subscription(&self) -> Subscription<DockAction> {
        match self.focused_split.as_ref().map(|focused| focused.2) {
            Some(DockAxis::Horizontal) => iced::event::listen_with(dock_horizontal_key_event),
            Some(DockAxis::Vertical) => iced::event::listen_with(dock_vertical_key_event),
            None => Subscription::none(),
        }
    }

    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.layout)
    }

    pub fn restore_layout_json(&mut self, value: &str) -> Result<Vec<DockHostEffect>, DockError> {
        let restored: DockLayout = serde_json::from_str(value)
            .map_err(|error| DockError::InvalidJson(error.to_string()))?;
        let restored = reconcile_layout(restored, &self.default_layout, &self.specs, &self.center)?;
        let effects = surface_diff(&self.layout, &restored);
        self.next_surface = restored
            .floating
            .iter()
            .map(|dock| dock.surface.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.layout = restored;
        self.active_resize = None;
        self.focused_split = None;
        self.active_drag = None;
        self.surface_bounds.retain(|surface, _| {
            *surface == DockSurfaceId(0)
                || self
                    .layout
                    .floating
                    .iter()
                    .any(|floating| floating.surface == *surface)
        });
        for floating in &self.layout.floating {
            self.surface_bounds
                .entry(floating.surface)
                .or_insert(floating.bounds);
        }
        Ok(effects)
    }

    /// Applies geometry and close events emitted by NanaUI's hosted runtime.
    #[cfg(feature = "hosted")]
    pub fn update_hosted_window(&mut self, event: HostedWindowEvent) -> DockUpdate {
        let surface = DockSurfaceId::from(event.id());
        match event {
            HostedWindowEvent::Ready { geometry, .. }
            | HostedWindowEvent::Resized { geometry, .. }
            | HostedWindowEvent::Moved { geometry, .. } => {
                let previous = self
                    .surface_bounds
                    .get(&surface)
                    .copied()
                    .or_else(|| {
                        self.layout
                            .floating
                            .iter()
                            .find(|floating| floating.surface == surface)
                            .map(|floating| floating.bounds)
                    })
                    .unwrap_or(DockBounds::new(
                        0.0,
                        0.0,
                        geometry.logical_size.width,
                        geometry.logical_size.height,
                    ));
                let position = geometry
                    .logical_position
                    .unwrap_or_else(|| Point::new(previous.x, previous.y));
                self.update(DockAction::SurfaceGeometry {
                    surface,
                    bounds: DockBounds::new(
                        position.x,
                        position.y,
                        geometry.logical_size.width,
                        geometry.logical_size.height,
                    ),
                })
            }
            HostedWindowEvent::CloseRequested { .. } if surface != DockSurfaceId(0) => {
                self.update(DockAction::CloseSurface(surface))
            }
            HostedWindowEvent::CloseRequested { .. }
            | HostedWindowEvent::VisibilityChanged { .. } => DockUpdate::default(),
        }
    }

    /// Reopens every floating surface already present in a restored layout.
    #[cfg(feature = "hosted")]
    pub fn open_hosted_windows(&self, title: impl Into<String>) -> HostedProgramUpdate {
        hosted_dock_update(
            DockUpdate {
                changed: false,
                effects: self
                    .layout
                    .floating
                    .iter()
                    .cloned()
                    .map(DockHostEffect::OpenFloating)
                    .collect(),
            },
            title,
        )
    }

    pub fn clamp_floating_bounds(
        &mut self,
        monitor_work_areas: &BTreeMap<String, DockBounds>,
        primary_work_area: DockBounds,
    ) -> bool {
        let mut changed = false;
        for floating in &mut self.layout.floating {
            let work_area = floating
                .monitor
                .as_ref()
                .and_then(|monitor| monitor_work_areas.get(monitor))
                .copied()
                .unwrap_or(primary_work_area);
            if floating
                .monitor
                .as_ref()
                .is_some_and(|monitor| !monitor_work_areas.contains_key(monitor))
            {
                floating.monitor = None;
                changed = true;
            }
            let bounds = floating.bounds.clamped_to(work_area);
            changed |= bounds != floating.bounds;
            floating.bounds = bounds;
        }
        changed
    }

    pub fn update(&mut self, action: DockAction) -> DockUpdate {
        if self.layout.locked
            && !matches!(
                action,
                DockAction::SetLocked(_)
                    | DockAction::Focus(_)
                    | DockAction::ActivateTab(_)
                    | DockAction::SurfaceResized { .. }
                    | DockAction::SurfaceGeometry { .. }
            )
        {
            return DockUpdate::default();
        }
        match action {
            DockAction::ActivateTab(id) => DockUpdate {
                changed: activate_tab_layout(&mut self.layout, &id),
                effects: Vec::new(),
            },
            DockAction::ReorderTab { id, before } => DockUpdate {
                changed: reorder_tab_layout(&mut self.layout, &id, before.as_ref()),
                effects: Vec::new(),
            },
            DockAction::ResizeStart { surface, path } => {
                let geometry = self.split_geometry(surface, &path);
                self.active_resize = geometry.map(|(axis, ratio, extent)| ActiveResize {
                    surface,
                    path: path.clone(),
                    drag: ResizeDrag::new(resize_axis(axis), ratio, 1.0 / extent),
                });
                self.focused_split = geometry.map(|(axis, _, _)| (surface, path, axis));
                DockUpdate::default()
            }
            DockAction::ResizeMove(position) => {
                let Some(active) = &mut self.active_resize else {
                    return DockUpdate::default();
                };
                let Some(ratio) = active.drag.value(position) else {
                    return DockUpdate::default();
                };
                let surface = active.surface;
                let path = active.path.clone();
                DockUpdate {
                    changed: self.set_surface_split_ratio(surface, &path, ratio),
                    effects: Vec::new(),
                }
            }
            DockAction::ResizeEnd => {
                self.active_resize = None;
                DockUpdate::default()
            }
            DockAction::ResizeSplit {
                surface,
                path,
                ratio,
            } => DockUpdate {
                changed: self.set_surface_split_ratio(surface, &path, ratio),
                effects: Vec::new(),
            },
            DockAction::AdjustSplit {
                surface,
                path,
                steps,
            } => {
                let Some((_, ratio, extent)) = self.split_geometry(surface, &path) else {
                    return DockUpdate::default();
                };
                DockUpdate {
                    changed: self.set_surface_split_ratio(
                        surface,
                        &path,
                        ratio + steps * 8.0 / extent.max(1.0),
                    ),
                    effects: Vec::new(),
                }
            }
            DockAction::KeyboardAdjust(steps) => {
                let Some((surface, path, _)) = self.focused_split.clone() else {
                    return DockUpdate::default();
                };
                self.update(DockAction::AdjustSplit {
                    surface,
                    path,
                    steps,
                })
            }
            DockAction::BlurSplit => {
                self.active_resize = None;
                self.focused_split = None;
                DockUpdate::default()
            }
            DockAction::ResetSplit { surface, path } => {
                let ratio = (surface == DockSurfaceId(0))
                    .then(|| split_at_path(&self.default_layout.main, &path))
                    .flatten()
                    .map(|(_, ratio)| ratio);
                DockUpdate {
                    changed: ratio
                        .is_some_and(|ratio| self.set_surface_split_ratio(surface, &path, ratio)),
                    effects: Vec::new(),
                }
            }
            DockAction::SurfaceResized {
                surface,
                width,
                height,
            } => {
                let size = (finite_positive(width, 1.0), finite_positive(height, 1.0));
                let previous = self
                    .surface_bounds
                    .get(&surface)
                    .copied()
                    .unwrap_or(DockBounds::new(0.0, 0.0, size.0, size.1));
                let bounds = DockBounds::new(previous.x, previous.y, size.0, size.1);
                let mut changed = self.surface_bounds.get(&surface) != Some(&bounds);
                self.surface_bounds.insert(surface, bounds);
                if let Some(floating) = self
                    .layout
                    .floating
                    .iter_mut()
                    .find(|floating| floating.surface == surface)
                {
                    changed |= floating.bounds.width != bounds.width
                        || floating.bounds.height != bounds.height;
                    floating.bounds.width = bounds.width;
                    floating.bounds.height = bounds.height;
                }
                DockUpdate {
                    changed,
                    effects: Vec::new(),
                }
            }
            DockAction::SurfaceGeometry { surface, bounds } => {
                if !valid_bounds(bounds) {
                    return DockUpdate::default();
                }
                let mut changed = self.surface_bounds.get(&surface) != Some(&bounds);
                self.surface_bounds.insert(surface, bounds);
                if let Some(floating) = self
                    .layout
                    .floating
                    .iter_mut()
                    .find(|floating| floating.surface == surface)
                {
                    changed |= floating.bounds != bounds;
                    floating.bounds = bounds;
                }
                DockUpdate {
                    changed,
                    effects: Vec::new(),
                }
            }
            DockAction::DragStart(id) => {
                if id == self.center || !self.is_visible(&id) {
                    return DockUpdate::default();
                }
                self.active_drag = Some(ActiveDrag {
                    id,
                    start: None,
                    position: None,
                    moved: false,
                    target: None,
                });
                DockUpdate::default()
            }
            DockAction::DragMove(position) => {
                let Some(mut drag) = self.active_drag.take() else {
                    return DockUpdate::default();
                };
                let start = *drag.start.get_or_insert(position);
                drag.moved |= (position.x - start.x)
                    .abs()
                    .max((position.y - start.y).abs())
                    >= 4.0;
                drag.position = Some(position);
                drag.target = self.drop_target_at(&drag.id, position);
                self.active_drag = Some(drag);
                DockUpdate::default()
            }
            DockAction::DragEnd => {
                let Some(drag) = self.active_drag.take() else {
                    return DockUpdate::default();
                };
                if !drag.moved {
                    return DockUpdate {
                        changed: activate_tab_layout(&mut self.layout, &drag.id),
                        effects: Vec::new(),
                    };
                }
                if let Some(target) = drag.target {
                    return self.dock(drag.id, target);
                }
                drag.position.map_or_else(DockUpdate::default, |position| {
                    self.float(
                        drag.id,
                        DockBounds::new(position.x - 180.0, position.y - 14.0, 360.0, 280.0),
                        None,
                    )
                })
            }
            DockAction::CancelDrag => {
                self.active_drag = None;
                DockUpdate::default()
            }
            DockAction::Hover(_) => DockUpdate::default(),
            DockAction::Hide(id) => self.hide(id),
            DockAction::Show(id) => self.show(id),
            DockAction::Float {
                id,
                bounds,
                monitor,
            } => self.float(id, bounds, monitor),
            DockAction::Dock { id, target } => self.dock(id, target),
            DockAction::Focus(id) => {
                let effect = self
                    .layout
                    .floating
                    .iter()
                    .find(|floating| floating.root.contains(&id))
                    .map(|floating| DockHostEffect::FocusFloating(floating.surface));
                DockUpdate {
                    changed: activate_tab_layout(&mut self.layout, &id),
                    effects: effect.into_iter().collect(),
                }
            }
            DockAction::CloseSurface(surface) => self.close_surface(surface),
            DockAction::SetLocked(locked) => {
                let changed = self.layout.locked != locked;
                self.layout.locked = locked;
                DockUpdate {
                    changed,
                    effects: Vec::new(),
                }
            }
            DockAction::Reset => {
                let effects = surface_diff(&self.layout, &self.default_layout);
                let changed = self.layout != self.default_layout;
                self.layout = self.default_layout.clone();
                self.active_resize = None;
                self.focused_split = None;
                self.active_drag = None;
                self.surface_bounds
                    .retain(|surface, _| *surface == DockSurfaceId(0));
                DockUpdate { changed, effects }
            }
        }
    }

    fn hide(&mut self, id: DockId) -> DockUpdate {
        if id == self.center || !self.specs.get(&id).is_some_and(|spec| spec.closeable) {
            return DockUpdate::default();
        }
        let (removed, surface) = remove_from_layout(&mut self.layout, &id);
        if !removed {
            return DockUpdate::default();
        }
        if !self.layout.hidden.contains(&id) {
            self.layout.hidden.push(id);
        }
        DockUpdate {
            changed: true,
            effects: surface
                .map(DockHostEffect::CloseFloating)
                .into_iter()
                .collect(),
        }
    }

    fn show(&mut self, id: DockId) -> DockUpdate {
        if id == self.center || !self.layout.hidden.contains(&id) || !self.specs.contains_key(&id) {
            return DockUpdate::default();
        }
        self.layout.hidden.retain(|hidden| hidden != &id);
        let target = first_non_center_id(&self.layout.main, &self.center)
            .unwrap_or_else(|| self.center.clone());
        insert_tab(&mut self.layout.main, &target, DockNode::item(id));
        DockUpdate {
            changed: true,
            effects: Vec::new(),
        }
    }

    fn float(&mut self, id: DockId, bounds: DockBounds, monitor: Option<String>) -> DockUpdate {
        if id == self.center || !self.specs.get(&id).is_some_and(|spec| spec.floatable) {
            return DockUpdate::default();
        }
        let (removed, closed_surface) = remove_from_layout(&mut self.layout, &id);
        if !removed {
            return DockUpdate::default();
        }
        self.layout.hidden.retain(|hidden| hidden != &id);
        let floating = FloatingDock {
            surface: DockSurfaceId(self.next_surface),
            root: DockNode::item(id),
            bounds,
            monitor,
        };
        self.next_surface = self.next_surface.saturating_add(1);
        self.layout.floating.push(floating.clone());
        self.surface_bounds.insert(floating.surface, bounds);
        let mut effects = closed_surface
            .map(DockHostEffect::CloseFloating)
            .into_iter()
            .collect::<Vec<_>>();
        effects.push(DockHostEffect::OpenFloating(floating));
        DockUpdate {
            changed: true,
            effects,
        }
    }

    fn dock(&mut self, id: DockId, target: DockDropTarget) -> DockUpdate {
        if id == self.center || id == target.id || !self.specs.contains_key(&id) {
            return DockUpdate::default();
        }
        let (removed, closed_surface) = remove_from_layout(&mut self.layout, &id);
        if !removed {
            return DockUpdate::default();
        }
        self.layout.hidden.retain(|hidden| hidden != &id);
        let node = DockNode::item(id);
        let target_root = if target.surface == DockSurfaceId(0) {
            Some(&mut self.layout.main)
        } else {
            self.layout
                .floating
                .iter_mut()
                .find(|floating| floating.surface == target.surface)
                .map(|floating| &mut floating.root)
        };
        let inserted = target_root.is_some_and(|root| match target.zone {
            DockDropZone::Tab => insert_tab(root, &target.id, node.clone()),
            zone => insert_split(root, &target.id, node.clone(), zone),
        });
        if !inserted {
            insert_tab(&mut self.layout.main, &self.center, node);
        }
        DockUpdate {
            changed: true,
            effects: closed_surface
                .map(DockHostEffect::CloseFloating)
                .into_iter()
                .collect(),
        }
    }

    fn close_surface(&mut self, surface: DockSurfaceId) -> DockUpdate {
        let Some(index) = self
            .layout
            .floating
            .iter()
            .position(|floating| floating.surface == surface)
        else {
            return DockUpdate::default();
        };
        let floating = self.layout.floating.remove(index);
        self.surface_bounds.remove(&surface);
        let mut ids = Vec::new();
        floating.root.ids(&mut ids);
        for id in ids {
            if self.specs.get(&id).is_some_and(|spec| spec.closeable)
                && !self.layout.hidden.contains(&id)
            {
                self.layout.hidden.push(id);
            }
        }
        DockUpdate {
            changed: true,
            effects: vec![DockHostEffect::CloseFloating(surface)],
        }
    }

    fn surface_root(&self, surface: DockSurfaceId) -> Option<&DockNode> {
        if surface == DockSurfaceId(0) {
            Some(&self.layout.main)
        } else {
            self.layout
                .floating
                .iter()
                .find(|floating| floating.surface == surface)
                .map(|floating| &floating.root)
        }
    }

    fn set_surface_split_ratio(
        &mut self,
        surface: DockSurfaceId,
        path: &[usize],
        ratio: f32,
    ) -> bool {
        let Some((axis, _, extent)) = self.split_geometry(surface, path) else {
            return false;
        };
        let Some((_, first, second)) = self
            .surface_root(surface)
            .and_then(|root| split_children_at_path(root, path))
            .map(|(axis, first, second)| (axis, first.clone(), second.clone()))
        else {
            return false;
        };
        let (first_min, first_max) = self.node_limits(&first, axis);
        let (second_min, second_max) = self.node_limits(&second, axis);
        let minimum = (first_min / extent)
            .max(second_max.map_or(MIN_SPLIT_RATIO, |maximum| 1.0 - maximum / extent))
            .clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
        let maximum = (1.0 - second_min / extent)
            .min(first_max.map_or(MAX_SPLIT_RATIO, |maximum| maximum / extent))
            .clamp(minimum, MAX_SPLIT_RATIO);
        let ratio = finite(ratio, 0.5).clamp(minimum, maximum);
        let root = if surface == DockSurfaceId(0) {
            Some(&mut self.layout.main)
        } else {
            self.layout
                .floating
                .iter_mut()
                .find(|floating| floating.surface == surface)
                .map(|floating| &mut floating.root)
        };
        root.is_some_and(|root| set_split_ratio(root, path, ratio))
    }

    fn split_geometry(
        &self,
        surface: DockSurfaceId,
        path: &[usize],
    ) -> Option<(DockAxis, f32, f32)> {
        let root = self.surface_root(surface)?;
        let bounds = self
            .surface_bounds
            .get(&surface)
            .copied()
            .unwrap_or(DockBounds::new(0.0, 0.0, 1280.0, 800.0));
        let bounds = split_bounds_at_path(root, bounds, path)?;
        let (axis, ratio) = split_at_path(root, path)?;
        let extent = match axis {
            DockAxis::Horizontal => bounds.width,
            DockAxis::Vertical => bounds.height,
        };
        Some((axis, ratio, (extent - DIVIDER_HIT_SIZE).max(1.0)))
    }

    fn node_limits(&self, node: &DockNode, axis: DockAxis) -> (f32, Option<f32>) {
        match node {
            DockNode::Item { id } => self.item_limits(id, axis),
            DockNode::Tabs { tabs, .. } => tabs.iter().fold((0.0_f32, None), |limits, id| {
                combine_parallel_limits(limits, self.item_limits(id, axis))
            }),
            DockNode::Split {
                axis: split_axis,
                first,
                second,
                ..
            } => {
                let first = self.node_limits(first, axis);
                let second = self.node_limits(second, axis);
                if *split_axis == axis {
                    (
                        first.0 + second.0 + DIVIDER_HIT_SIZE,
                        match (first.1, second.1) {
                            (Some(first), Some(second)) => Some(first + second + DIVIDER_HIT_SIZE),
                            _ => None,
                        },
                    )
                } else {
                    combine_parallel_limits(first, second)
                }
            }
        }
    }

    fn item_limits(&self, id: &DockId, axis: DockAxis) -> (f32, Option<f32>) {
        self.specs.get(id).map_or((0.0, None), |spec| {
            if axis == DockAxis::Horizontal {
                (spec.minimum_width, spec.maximum_width)
            } else {
                (spec.minimum_height, spec.maximum_height)
            }
        })
    }

    fn drop_target_at(&self, dragged: &DockId, position: Point) -> Option<DockDropTarget> {
        let mut surfaces = self
            .layout
            .floating
            .iter()
            .rev()
            .map(|floating| (floating.surface, &floating.root))
            .collect::<Vec<_>>();
        surfaces.push((DockSurfaceId(0), &self.layout.main));
        for (surface, root) in surfaces {
            let Some(bounds) = self.surface_bounds.get(&surface).copied() else {
                continue;
            };
            if !bounds_contains(bounds, position) {
                continue;
            }
            let mut targets = Vec::new();
            collect_drop_targets(root, bounds, &mut targets);
            if let Some((id, bounds)) = targets
                .into_iter()
                .find(|(id, bounds)| id != dragged && bounds_contains(*bounds, position))
            {
                let local_x = (position.x - bounds.x) / bounds.width.max(1.0);
                let local_y = (position.y - bounds.y) / bounds.height.max(1.0);
                let zone = if local_x <= 0.25 {
                    DockDropZone::Left
                } else if local_x >= 0.75 {
                    DockDropZone::Right
                } else if local_y <= 0.25 {
                    DockDropZone::Top
                } else if local_y >= 0.75 {
                    DockDropZone::Bottom
                } else if id == self.center {
                    continue;
                } else {
                    DockDropZone::Tab
                };
                return Some(DockDropTarget { surface, id, zone });
            }
        }
        None
    }
}

/// Converts Dock window effects into commands understood by [`crate::run_hosted`].
#[cfg(feature = "hosted")]
pub fn hosted_dock_update(update: DockUpdate, title: impl Into<String>) -> HostedProgramUpdate {
    let title = title.into();
    let commands = update.effects.into_iter().map(|effect| match effect {
        DockHostEffect::OpenFloating(floating) => HostedWindowCommand::Open {
            id: HostedWindowId::from(floating.surface),
            settings: HostedWindowSettings::new(title.clone())
                .tool_window()
                .initial_position(f64::from(floating.bounds.x), f64::from(floating.bounds.y))
                .initial_size(
                    f64::from(floating.bounds.width),
                    f64::from(floating.bounds.height),
                )
                .minimum_size(160.0, 120.0),
        },
        DockHostEffect::CloseFloating(surface) => {
            HostedWindowCommand::Close(HostedWindowId::from(surface))
        }
        DockHostEffect::FocusFloating(surface) => {
            HostedWindowCommand::Focus(HostedWindowId::from(surface))
        }
    });
    HostedProgramUpdate::redraw_all().with_window_commands(commands)
}

/// Application-owned contents keyed by stable dock ID.
pub struct DockContents<'a, Message> {
    items: BTreeMap<DockId, Element<'a, Message>>,
}

impl<'a, Message> DockContents<'a, Message> {
    pub fn new() -> Self {
        Self {
            items: BTreeMap::new(),
        }
    }

    pub fn insert(
        mut self,
        id: impl Into<DockId>,
        content: impl Into<Element<'a, Message>>,
    ) -> Self {
        self.items.insert(id.into(), content.into());
        self
    }
}

impl<Message> Default for DockContents<'_, Message> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
struct DockSurfaceState {
    bounds: Option<DockBounds>,
}

struct DockSurface<'a, Message> {
    content: Element<'a, Message>,
    on_geometry: Rc<dyn Fn(DockBounds) -> Message + 'a>,
}

impl<'a, Message> DockSurface<'a, Message> {
    fn new(
        content: impl Into<Element<'a, Message>>,
        on_geometry: impl Fn(DockBounds) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            on_geometry: Rc::new(on_geometry),
        }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for DockSurface<'_, Message>
where
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<DockSurfaceState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(DockSurfaceState::default())
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(&mut [self.content.as_widget_mut()]);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let content = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(content.size(), vec![content])
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let bounds = DockBounds::new(bounds.x, bounds.y, bounds.width, bounds.height);
        let state = tree.state.downcast_mut::<DockSurfaceState>();
        if state.bounds != Some(bounds) {
            state.bounds = Some(bounds);
            shell.publish((self.on_geometry)(bounds));
        }
        let content_layout = layout.children().next().expect("dock surface content");
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            content_layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let content_layout = layout.children().next().expect("dock surface content");
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            content_layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let content_layout = layout.children().next().expect("dock surface content");
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            content_layout,
            renderer,
            operation,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let content_layout = layout.children().next().expect("dock surface content");
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            content_layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        let content_layout = layout.children().next().expect("dock surface content");
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            content_layout,
            renderer,
            viewport,
            translation,
        )
    }
}

/// Renders one dock surface. Floating surfaces use the same controller and a different root.
pub fn dock_workspace<'a, Message>(
    controller: &'a DockController,
    surface: DockSurfaceId,
    mut contents: DockContents<'a, Message>,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    theme: impl Into<ThemeTokens>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let root = if surface == DockSurfaceId(0) {
        Some(&controller.layout.main)
    } else {
        controller
            .layout
            .floating
            .iter()
            .find(|floating| floating.surface == surface)
            .map(|floating| &floating.root)
    };
    let content = root.map_or_else(
        || {
            container(space())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        },
        |root| {
            dock_node_view(
                root,
                surface,
                Vec::new(),
                controller,
                &mut contents,
                on_action,
                theme.into(),
            )
        },
    );
    Element::new(DockSurface::new(content, move |bounds| {
        on_action(DockAction::SurfaceResized {
            surface,
            width: bounds.width,
            height: bounds.height,
        })
    }))
}

fn dock_node_view<'a, Message>(
    node: &'a DockNode,
    surface: DockSurfaceId,
    path: Vec<usize>,
    controller: &'a DockController,
    contents: &mut DockContents<'a, Message>,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    match node {
        DockNode::Item { id } => dock_item_view(id, false, controller, contents, on_action, tokens),
        DockNode::Tabs { tabs, active } => {
            let tab_bar = tabs.iter().fold(
                row![].height(Length::Fixed(TITLE_BAR_HEIGHT)),
                |tabs_row, id| {
                    let title = controller
                        .item(id)
                        .map_or_else(|| id.as_str(), |spec| spec.title.as_str());
                    let tab = container(
                        text(title)
                            .size(11)
                            .font(ui_font(iced::font::Weight::Medium)),
                    )
                    .center_y(Length::Fill)
                    .padding([0.0, 10.0])
                    .style(move |_theme| {
                        iced::widget::container::Style::default()
                            .background(if id == active {
                                tokens.colors.active
                            } else {
                                tokens.colors.surface
                            })
                            .border(iced::Border {
                                color: tokens.colors.border,
                                width: 1.0,
                                radius: 0.0.into(),
                            })
                    });
                    if controller.layout.locked {
                        tabs_row.push(
                            button(tab)
                                .height(Length::Fixed(TITLE_BAR_HEIGHT))
                                .padding(0)
                                .on_press(on_action(DockAction::ActivateTab(id.clone())))
                                .style(button_style(tokens, ButtonKind::Text)),
                        )
                    } else {
                        tabs_row.push(DragHandle::new(
                            tab.height(Length::Fixed(TITLE_BAR_HEIGHT))
                                .width(Length::Shrink),
                            on_action(DockAction::DragStart(id.clone())),
                            move |point| on_action(DockAction::DragMove(point)),
                            on_action(DockAction::DragEnd),
                            on_action(DockAction::ActivateTab(id.clone())),
                            move |hovered| on_action(DockAction::Hover(hovered)),
                            iced::mouse::Interaction::Grabbing,
                        ))
                    }
                },
            );
            column![
                container(tab_bar)
                    .width(Length::Fill)
                    .height(Length::Fixed(TITLE_BAR_HEIGHT))
                    .style(move |_theme| iced::widget::container::Style::default()
                        .background(tokens.colors.surface)
                        .border(iced::Border {
                            color: tokens.colors.border,
                            width: 1.0,
                            radius: 0.0.into(),
                        })),
                dock_item_view(active, true, controller, contents, on_action, tokens),
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        }
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let mut first_path = path.clone();
            first_path.push(0);
            let mut second_path = path.clone();
            second_path.push(1);
            let first = dock_node_view(
                first, surface, first_path, controller, contents, on_action, tokens,
            );
            let second = dock_node_view(
                second,
                surface,
                second_path,
                controller,
                contents,
                on_action,
                tokens,
            );
            let divider_path = path;
            let indicator = container(space())
                .width(if *axis == DockAxis::Horizontal {
                    Length::Fixed(1.0)
                } else {
                    Length::Fill
                })
                .height(if *axis == DockAxis::Horizontal {
                    Length::Fill
                } else {
                    Length::Fixed(1.0)
                })
                .style(move |_theme| {
                    iced::widget::container::Style::default().background(tokens.colors.border)
                });
            let divider = container(indicator)
                .width(if *axis == DockAxis::Horizontal {
                    Length::Fixed(DIVIDER_HIT_SIZE)
                } else {
                    Length::Fill
                })
                .height(if *axis == DockAxis::Horizontal {
                    Length::Fill
                } else {
                    Length::Fixed(DIVIDER_HIT_SIZE)
                })
                .center(Length::Fill);
            let divider = DragHandle::new(
                divider,
                on_action(DockAction::ResizeStart {
                    surface,
                    path: divider_path.clone(),
                }),
                move |point| on_action(DockAction::ResizeMove(point)),
                on_action(DockAction::ResizeEnd),
                on_action(DockAction::ResetSplit {
                    surface,
                    path: divider_path,
                }),
                move |_| on_action(DockAction::ResizeEnd),
                if *axis == DockAxis::Horizontal {
                    iced::mouse::Interaction::ResizingHorizontally
                } else {
                    iced::mouse::Interaction::ResizingVertically
                },
            );
            let first_portion = ((*ratio * 10_000.0).round() as u16).max(1);
            let second_portion = 10_000_u16.saturating_sub(first_portion).max(1);
            let first = container(first)
                .width(if *axis == DockAxis::Horizontal {
                    Length::FillPortion(first_portion)
                } else {
                    Length::Fill
                })
                .height(if *axis == DockAxis::Vertical {
                    Length::FillPortion(first_portion)
                } else {
                    Length::Fill
                })
                .clip(true);
            let second = container(second)
                .width(if *axis == DockAxis::Horizontal {
                    Length::FillPortion(second_portion)
                } else {
                    Length::Fill
                })
                .height(if *axis == DockAxis::Vertical {
                    Length::FillPortion(second_portion)
                } else {
                    Length::Fill
                })
                .clip(true);
            if *axis == DockAxis::Horizontal {
                row![first, divider, second]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                column![first, divider, second]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        }
    }
}

fn dock_item_view<'a, Message>(
    id: &'a DockId,
    tabs_own_title: bool,
    controller: &'a DockController,
    contents: &mut DockContents<'a, Message>,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let spec = controller.item(id);
    let title = spec.map_or_else(|| id.as_str(), |spec| spec.title.as_str());
    let title = container(
        text(title)
            .size(11)
            .font(ui_font(iced::font::Weight::Semibold)),
    )
    .width(Length::Fill)
    .height(Length::Fixed(TITLE_BAR_HEIGHT))
    .center_y(Length::Fill);
    let title: Element<'a, Message> = if controller.layout.locked || id == &controller.center {
        title.into()
    } else {
        DragHandle::new(
            title,
            on_action(DockAction::DragStart(id.clone())),
            move |point| on_action(DockAction::DragMove(point)),
            on_action(DockAction::DragEnd),
            on_action(DockAction::Focus(id.clone())),
            move |hovered| on_action(DockAction::Hover(hovered)),
            iced::mouse::Interaction::Grabbing,
        )
        .into()
    };
    let mut title_bar = row![title].spacing(4).align_y(Alignment::Center);
    if !controller.layout.locked && id != &controller.center {
        if spec.is_some_and(|spec| spec.floatable) {
            title_bar = title_bar.push(
                button(text("↗").size(11))
                    .width(Length::Fixed(24.0))
                    .height(Length::Fixed(24.0))
                    .padding(0)
                    .on_press(on_action(DockAction::Float {
                        id: id.clone(),
                        bounds: DockBounds::new(120.0, 120.0, 360.0, 280.0),
                        monitor: None,
                    }))
                    .style(button_style(tokens, ButtonKind::Text)),
            );
        }
        if spec.is_some_and(|spec| spec.closeable) {
            title_bar = title_bar.push(
                button(text("×").size(13))
                    .width(Length::Fixed(24.0))
                    .height(Length::Fixed(24.0))
                    .padding(0)
                    .on_press(on_action(DockAction::Hide(id.clone())))
                    .style(button_style(tokens, ButtonKind::Text)),
            );
        }
    }
    let content = contents
        .items
        .remove(id)
        .unwrap_or_else(|| container(space()).into());
    let content: Element<'a, Message> = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(tokens.colors.surface)
                .border(iced::Border {
                    color: tokens.colors.border,
                    width: 1.0,
                    radius: 0.0.into(),
                })
        })
        .into();
    let content = if let Some(target) = controller.drop_target().filter(|target| target.id == *id) {
        let accent = iced::Color {
            a: 0.22,
            ..tokens.colors.accent
        };
        let mut overlay = container(space()).style(move |_theme| {
            iced::widget::container::Style::default()
                .background(accent)
                .border(iced::Border {
                    color: tokens.colors.accent,
                    width: 2.0,
                    radius: 0.0.into(),
                })
        });
        overlay = match target.zone {
            DockDropZone::Left | DockDropZone::Right => {
                overlay.width(Length::FillPortion(1)).height(Length::Fill)
            }
            DockDropZone::Top | DockDropZone::Bottom => {
                overlay.width(Length::Fill).height(Length::FillPortion(1))
            }
            DockDropZone::Tab => overlay.width(Length::Fill).height(Length::Fill),
        };
        let overlay = match target.zone {
            DockDropZone::Left => container(overlay)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_left(Length::Fill),
            DockDropZone::Right => container(overlay)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_right(Length::Fill),
            DockDropZone::Top => container(overlay)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_top(Length::Fill),
            DockDropZone::Bottom => container(overlay)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_bottom(Length::Fill),
            DockDropZone::Tab => container(overlay)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(16),
        };
        stack![content, overlay]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        content
    };
    if tabs_own_title || id == &controller.center {
        content
    } else {
        column![
            container(title_bar)
                .width(Length::Fill)
                .height(Length::Fixed(TITLE_BAR_HEIGHT))
                .padding([0.0, 6.0])
                .style(move |_theme| iced::widget::container::Style::default()
                    .background(tokens.colors.surface)
                    .border(iced::Border {
                        color: tokens.colors.border,
                        width: 1.0,
                        radius: 0.0.into(),
                    })),
            content,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

fn validate_layout(
    layout: &DockLayout,
    specs: &BTreeMap<DockId, DockItemSpec>,
    center: &DockId,
) -> Result<(), DockError> {
    if layout.version != DOCK_LAYOUT_VERSION {
        return Err(DockError::UnsupportedVersion(layout.version));
    }
    let mut ids = Vec::new();
    validate_node(&layout.main, &mut ids)?;
    for floating in &layout.floating {
        validate_node(&floating.root, &mut ids)?;
        if floating.root.contains(center) {
            return Err(DockError::InvalidCenter(center.clone()));
        }
        if !valid_bounds(floating.bounds) {
            return Err(DockError::InvalidSplit);
        }
    }
    for hidden in &layout.hidden {
        ids.push(hidden.clone());
    }
    let mut seen = BTreeSet::new();
    for id in ids {
        if !specs.contains_key(&id) {
            return Err(DockError::UnknownDock(id));
        }
        if !seen.insert(id.clone()) {
            return Err(DockError::DuplicateDock(id));
        }
    }
    if !layout.main.contains(center) {
        return Err(DockError::MissingCenter(center.clone()));
    }
    if contains_center_in_tabs(&layout.main, center) {
        return Err(DockError::InvalidCenter(center.clone()));
    }
    Ok(())
}

fn validate_node(node: &DockNode, ids: &mut Vec<DockId>) -> Result<(), DockError> {
    match node {
        DockNode::Item { id } => ids.push(id.clone()),
        DockNode::Tabs { tabs, active } => {
            if tabs.is_empty() || !tabs.contains(active) {
                return Err(DockError::InvalidTabs);
            }
            ids.extend(tabs.iter().cloned());
        }
        DockNode::Split {
            ratio,
            first,
            second,
            ..
        } => {
            if !ratio.is_finite() || !(MIN_SPLIT_RATIO..=MAX_SPLIT_RATIO).contains(ratio) {
                return Err(DockError::InvalidSplit);
            }
            validate_node(first, ids)?;
            validate_node(second, ids)?;
        }
    }
    Ok(())
}

fn reconcile_layout(
    mut restored: DockLayout,
    default: &DockLayout,
    specs: &BTreeMap<DockId, DockItemSpec>,
    center: &DockId,
) -> Result<DockLayout, DockError> {
    if restored.version != DOCK_LAYOUT_VERSION {
        return Err(DockError::UnsupportedVersion(restored.version));
    }
    restored.main = prune_unknown(restored.main, specs).unwrap_or_else(|| default.main.clone());
    restored.floating.retain_mut(|floating| {
        let Some(root) = prune_unknown(floating.root.clone(), specs) else {
            return false;
        };
        floating.root = root;
        valid_bounds(floating.bounds)
    });
    restored.hidden.retain(|id| specs.contains_key(id));

    let mut present = Vec::new();
    restored.main.ids(&mut present);
    for floating in &restored.floating {
        floating.root.ids(&mut present);
    }
    present.extend(restored.hidden.iter().cloned());
    let mut seen = BTreeSet::new();
    if let Some(duplicate) = present.iter().find(|id| !seen.insert((*id).clone())) {
        return Err(DockError::DuplicateDock(duplicate.clone()));
    }
    if !restored.main.contains(center) || contains_center_in_tabs(&restored.main, center) {
        restored.main = default.main.clone();
        restored
            .floating
            .retain(|floating| !floating.root.contains(center));
        restored.hidden.retain(|id| id != center);
    }

    let mut current = Vec::new();
    restored.main.ids(&mut current);
    for floating in &restored.floating {
        floating.root.ids(&mut current);
    }
    current.extend(restored.hidden.iter().cloned());
    let current = current.into_iter().collect::<BTreeSet<_>>();
    let mut defaults = Vec::new();
    default.main.ids(&mut defaults);
    for floating in &default.floating {
        floating.root.ids(&mut defaults);
    }
    defaults.extend(default.hidden.iter().cloned());
    for id in defaults {
        if !current.contains(&id) && id != *center {
            insert_tab(&mut restored.main, center, DockNode::item(id));
        }
    }
    validate_layout(&restored, specs, center)?;
    Ok(restored)
}

fn prune_unknown(node: DockNode, specs: &BTreeMap<DockId, DockItemSpec>) -> Option<DockNode> {
    match node {
        DockNode::Item { id } => specs.contains_key(&id).then_some(DockNode::Item { id }),
        DockNode::Tabs { mut tabs, active } => {
            tabs.retain(|id| specs.contains_key(id));
            match tabs.len() {
                0 => None,
                1 => Some(DockNode::item(tabs.remove(0))),
                _ => {
                    let active = if tabs.contains(&active) {
                        active
                    } else {
                        tabs[0].clone()
                    };
                    Some(DockNode::Tabs { tabs, active })
                }
            }
        }
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => match (prune_unknown(*first, specs), prune_unknown(*second, specs)) {
            (Some(first), Some(second)) => Some(DockNode::split(axis, ratio, first, second)),
            (Some(node), None) | (None, Some(node)) => Some(node),
            (None, None) => None,
        },
    }
}

fn contains_center_in_tabs(node: &DockNode, center: &DockId) -> bool {
    match node {
        DockNode::Item { .. } => false,
        DockNode::Tabs { tabs, .. } => tabs.contains(center),
        DockNode::Split { first, second, .. } => {
            contains_center_in_tabs(first, center) || contains_center_in_tabs(second, center)
        }
    }
}

fn surface_diff(before: &DockLayout, after: &DockLayout) -> Vec<DockHostEffect> {
    let before_surfaces = before
        .floating
        .iter()
        .map(|dock| dock.surface)
        .collect::<BTreeSet<_>>();
    let after_surfaces = after
        .floating
        .iter()
        .map(|dock| dock.surface)
        .collect::<BTreeSet<_>>();
    let mut effects = before_surfaces
        .difference(&after_surfaces)
        .copied()
        .map(DockHostEffect::CloseFloating)
        .collect::<Vec<_>>();
    effects.extend(
        after
            .floating
            .iter()
            .filter(|dock| !before_surfaces.contains(&dock.surface))
            .cloned()
            .map(DockHostEffect::OpenFloating),
    );
    effects
}

fn activate_tab_layout(layout: &mut DockLayout, id: &DockId) -> bool {
    activate_tab(&mut layout.main, id)
        || layout
            .floating
            .iter_mut()
            .any(|floating| activate_tab(&mut floating.root, id))
}

fn activate_tab(node: &mut DockNode, id: &DockId) -> bool {
    match node {
        DockNode::Item { .. } => false,
        DockNode::Tabs { tabs, active } => {
            if tabs.contains(id) && active != id {
                *active = id.clone();
                true
            } else {
                false
            }
        }
        DockNode::Split { first, second, .. } => {
            activate_tab(first, id) || activate_tab(second, id)
        }
    }
}

fn reorder_tab_layout(layout: &mut DockLayout, id: &DockId, before: Option<&DockId>) -> bool {
    reorder_tab(&mut layout.main, id, before)
        || layout
            .floating
            .iter_mut()
            .any(|floating| reorder_tab(&mut floating.root, id, before))
}

fn reorder_tab(node: &mut DockNode, id: &DockId, before: Option<&DockId>) -> bool {
    match node {
        DockNode::Tabs { tabs, .. } if tabs.contains(id) => {
            let old = tabs.iter().position(|tab| tab == id).unwrap_or_default();
            let item = tabs.remove(old);
            let new = before
                .and_then(|before| tabs.iter().position(|tab| tab == before))
                .unwrap_or(tabs.len());
            tabs.insert(new, item);
            old != new
        }
        DockNode::Split { first, second, .. } => {
            reorder_tab(first, id, before) || reorder_tab(second, id, before)
        }
        _ => false,
    }
}

fn split_at_path(node: &DockNode, path: &[usize]) -> Option<(DockAxis, f32)> {
    if path.is_empty() {
        return match node {
            DockNode::Split { axis, ratio, .. } => Some((*axis, *ratio)),
            _ => None,
        };
    }
    let DockNode::Split { first, second, .. } = node else {
        return None;
    };
    match path[0] {
        0 => split_at_path(first, &path[1..]),
        1 => split_at_path(second, &path[1..]),
        _ => None,
    }
}

fn split_children_at_path<'a>(
    node: &'a DockNode,
    path: &[usize],
) -> Option<(DockAxis, &'a DockNode, &'a DockNode)> {
    if path.is_empty() {
        return match node {
            DockNode::Split {
                axis,
                first,
                second,
                ..
            } => Some((*axis, first, second)),
            _ => None,
        };
    }
    let DockNode::Split { first, second, .. } = node else {
        return None;
    };
    match path[0] {
        0 => split_children_at_path(first, &path[1..]),
        1 => split_children_at_path(second, &path[1..]),
        _ => None,
    }
}

fn split_bounds_at_path(node: &DockNode, bounds: DockBounds, path: &[usize]) -> Option<DockBounds> {
    if path.is_empty() {
        return matches!(node, DockNode::Split { .. }).then_some(bounds);
    }
    let DockNode::Split {
        axis,
        ratio,
        first,
        second,
    } = node
    else {
        return None;
    };
    let (first_bounds, second_bounds) = split_child_bounds(*axis, *ratio, bounds);
    match path[0] {
        0 => split_bounds_at_path(first, first_bounds, &path[1..]),
        1 => split_bounds_at_path(second, second_bounds, &path[1..]),
        _ => None,
    }
}

fn split_child_bounds(axis: DockAxis, ratio: f32, bounds: DockBounds) -> (DockBounds, DockBounds) {
    match axis {
        DockAxis::Horizontal => {
            let first_width = (bounds.width - DIVIDER_HIT_SIZE).max(0.0) * ratio;
            (
                DockBounds::new(bounds.x, bounds.y, first_width, bounds.height),
                DockBounds::new(
                    bounds.x + first_width + DIVIDER_HIT_SIZE,
                    bounds.y,
                    (bounds.width - first_width - DIVIDER_HIT_SIZE).max(0.0),
                    bounds.height,
                ),
            )
        }
        DockAxis::Vertical => {
            let first_height = (bounds.height - DIVIDER_HIT_SIZE).max(0.0) * ratio;
            (
                DockBounds::new(bounds.x, bounds.y, bounds.width, first_height),
                DockBounds::new(
                    bounds.x,
                    bounds.y + first_height + DIVIDER_HIT_SIZE,
                    bounds.width,
                    (bounds.height - first_height - DIVIDER_HIT_SIZE).max(0.0),
                ),
            )
        }
    }
}

fn combine_parallel_limits(
    first: (f32, Option<f32>),
    second: (f32, Option<f32>),
) -> (f32, Option<f32>) {
    (
        first.0.max(second.0),
        match (first.1, second.1) {
            (Some(first), Some(second)) => Some(first.max(second)),
            _ => None,
        },
    )
}

fn collect_drop_targets(
    node: &DockNode,
    bounds: DockBounds,
    output: &mut Vec<(DockId, DockBounds)>,
) {
    match node {
        DockNode::Item { id } => output.push((id.clone(), bounds)),
        DockNode::Tabs { active, .. } => output.push((active.clone(), bounds)),
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let (first_bounds, second_bounds) = split_child_bounds(*axis, *ratio, bounds);
            collect_drop_targets(first, first_bounds, output);
            collect_drop_targets(second, second_bounds, output);
        }
    }
}

fn bounds_contains(bounds: DockBounds, point: Point) -> bool {
    point.x >= bounds.x
        && point.y >= bounds.y
        && point.x <= bounds.x + bounds.width
        && point.y <= bounds.y + bounds.height
}

fn dock_horizontal_key_event(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<DockAction> {
    dock_key_event(event, status, DockAxis::Horizontal)
}

fn dock_vertical_key_event(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<DockAction> {
    dock_key_event(event, status, DockAxis::Vertical)
}

fn dock_key_event(
    event: iced::Event,
    status: iced::event::Status,
    axis: DockAxis,
) -> Option<DockAction> {
    if status == iced::event::Status::Captured {
        return None;
    }
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    use iced::keyboard::key::Named;
    match (axis, key) {
        (_, iced::keyboard::Key::Named(Named::Escape)) => Some(DockAction::BlurSplit),
        (DockAxis::Horizontal, iced::keyboard::Key::Named(Named::ArrowLeft))
        | (DockAxis::Vertical, iced::keyboard::Key::Named(Named::ArrowUp)) => {
            Some(DockAction::KeyboardAdjust(-1.0))
        }
        (DockAxis::Horizontal, iced::keyboard::Key::Named(Named::ArrowRight))
        | (DockAxis::Vertical, iced::keyboard::Key::Named(Named::ArrowDown)) => {
            Some(DockAction::KeyboardAdjust(1.0))
        }
        _ => None,
    }
}

fn set_split_ratio(node: &mut DockNode, path: &[usize], ratio: f32) -> bool {
    if path.is_empty() {
        let DockNode::Split { ratio: current, .. } = node else {
            return false;
        };
        let ratio = clamp_ratio(ratio);
        let changed = *current != ratio;
        *current = ratio;
        return changed;
    }
    let DockNode::Split { first, second, .. } = node else {
        return false;
    };
    match path[0] {
        0 => set_split_ratio(first, &path[1..], ratio),
        1 => set_split_ratio(second, &path[1..], ratio),
        _ => false,
    }
}

fn remove_from_layout(layout: &mut DockLayout, id: &DockId) -> (bool, Option<DockSurfaceId>) {
    if let Some(root) = remove_node(layout.main.clone(), id)
        && root != layout.main
    {
        layout.main = root;
        return (true, None);
    }
    if let Some(index) = layout
        .floating
        .iter()
        .position(|floating| floating.root.contains(id))
    {
        let surface = layout.floating[index].surface;
        match remove_node(layout.floating[index].root.clone(), id) {
            Some(root) if root != layout.floating[index].root => {
                layout.floating[index].root = root;
                (true, None)
            }
            None => {
                layout.floating.remove(index);
                (true, Some(surface))
            }
            _ => (false, None),
        }
    } else {
        (false, None)
    }
}

fn remove_node(node: DockNode, id: &DockId) -> Option<DockNode> {
    match node {
        DockNode::Item { id: item } => (item != *id).then_some(DockNode::item(item)),
        DockNode::Tabs {
            mut tabs,
            mut active,
        } => {
            let before = tabs.len();
            tabs.retain(|tab| tab != id);
            if tabs.len() == before {
                return Some(DockNode::Tabs { tabs, active });
            }
            match tabs.len() {
                0 => None,
                1 => Some(DockNode::item(tabs.remove(0))),
                _ => {
                    if active == *id {
                        active = tabs[0].clone();
                    }
                    Some(DockNode::Tabs { tabs, active })
                }
            }
        }
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => match (remove_node(*first, id), remove_node(*second, id)) {
            (Some(first), Some(second)) => Some(DockNode::split(axis, ratio, first, second)),
            (Some(node), None) | (None, Some(node)) => Some(node),
            (None, None) => None,
        },
    }
}

fn insert_tab(root: &mut DockNode, target: &DockId, node: DockNode) -> bool {
    let mut ids = Vec::new();
    node.ids(&mut ids);
    let Some(id) = ids.into_iter().next() else {
        return false;
    };
    match root {
        DockNode::Item { id: current } if current == target => {
            let current = current.clone();
            *root = DockNode::Tabs {
                tabs: vec![current, id.clone()],
                active: id,
            };
            true
        }
        DockNode::Tabs { tabs, active } if tabs.contains(target) => {
            if !tabs.contains(&id) {
                tabs.push(id.clone());
            }
            *active = id;
            true
        }
        DockNode::Split { first, second, .. } => {
            insert_tab(first, target, node.clone()) || insert_tab(second, target, node)
        }
        _ => false,
    }
}

fn insert_split(root: &mut DockNode, target: &DockId, node: DockNode, zone: DockDropZone) -> bool {
    if root.contains(target) && matches!(root, DockNode::Item { .. } | DockNode::Tabs { .. }) {
        let previous = root.clone();
        let (axis, first, second) = match zone {
            DockDropZone::Left => (DockAxis::Horizontal, node, previous),
            DockDropZone::Right => (DockAxis::Horizontal, previous, node),
            DockDropZone::Top => (DockAxis::Vertical, node, previous),
            DockDropZone::Bottom => (DockAxis::Vertical, previous, node),
            DockDropZone::Tab => return insert_tab(root, target, node),
        };
        *root = DockNode::split(axis, 0.5, first, second);
        return true;
    }
    match root {
        DockNode::Split { first, second, .. } => {
            insert_split(first, target, node.clone(), zone)
                || insert_split(second, target, node, zone)
        }
        _ => false,
    }
}

fn first_non_center_id(node: &DockNode, center: &DockId) -> Option<DockId> {
    match node {
        DockNode::Item { id } => (id != center).then(|| id.clone()),
        DockNode::Tabs { tabs, .. } => tabs.iter().find(|id| *id != center).cloned(),
        DockNode::Split { first, second, .. } => {
            first_non_center_id(first, center).or_else(|| first_non_center_id(second, center))
        }
    }
}

fn valid_bounds(bounds: DockBounds) -> bool {
    bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
        && bounds.width > 0.0
        && bounds.height > 0.0
}

fn clamp_ratio(ratio: f32) -> f32 {
    finite(ratio, 0.5).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn finite_positive(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

fn resize_axis(axis: DockAxis) -> ResizeAxis {
    match axis {
        DockAxis::Horizontal => ResizeAxis::Horizontal,
        DockAxis::Vertical => ResizeAxis::Vertical,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "hosted")]
    use crate::HostedWindowGeometry;

    fn controller() -> DockController {
        let layout = DockLayout::new(DockNode::split(
            DockAxis::Horizontal,
            0.25,
            DockNode::tabs([DockId::from("scenes"), DockId::from("sources")], "scenes"),
            DockNode::split(
                DockAxis::Vertical,
                0.75,
                DockNode::item("editor"),
                DockNode::tabs([DockId::from("mixer"), DockId::from("controls")], "mixer"),
            ),
        ));
        DockController::new(
            "editor",
            [
                DockItemSpec::new("editor", "Editor").limits(360.0, 240.0),
                DockItemSpec::new("scenes", "Scenes").limits(150.0, 120.0),
                DockItemSpec::new("sources", "Sources").limits(150.0, 120.0),
                DockItemSpec::new("mixer", "Mixer"),
                DockItemSpec::new("controls", "Controls"),
            ],
            layout,
        )
        .expect("valid dock layout")
    }

    #[test]
    fn center_cannot_be_hidden_floated_or_tabbed() {
        let mut controller = controller();
        assert!(!controller.update(DockAction::Hide("editor".into())).changed);
        assert!(
            !controller
                .update(DockAction::Float {
                    id: "editor".into(),
                    bounds: DockBounds::new(0.0, 0.0, 300.0, 200.0),
                    monitor: None,
                })
                .changed
        );
        assert!(controller.is_visible(&DockId::from("editor")));
    }

    #[test]
    fn floating_and_redocking_emit_host_effects() {
        let mut controller = controller();
        let update = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(40.0, 50.0, 360.0, 280.0),
            monitor: Some("display-2".into()),
        });
        assert!(update.changed);
        let DockHostEffect::OpenFloating(floating) = &update.effects[0] else {
            panic!("floating window open effect")
        };
        let surface = floating.surface;
        let update = controller.update(DockAction::Dock {
            id: "sources".into(),
            target: DockDropTarget {
                surface: DockSurfaceId(0),
                id: "scenes".into(),
                zone: DockDropZone::Tab,
            },
        });
        assert_eq!(update.effects, vec![DockHostEffect::CloseFloating(surface)]);
        assert!(controller.is_visible(&DockId::from("sources")));
    }

    #[test]
    fn layout_round_trip_rejects_duplicates_and_restores_new_registered_docks() {
        let mut state = controller();
        state.update(DockAction::Hide("controls".into()));
        let encoded = state.layout_json().expect("dock layout serializes");
        let mut restored = controller();
        restored
            .restore_layout_json(&encoded)
            .expect("dock layout restores");
        assert!(!restored.is_visible(&DockId::from("controls")));

        let duplicate = encoded.replace(
            "\"hidden\":[\"controls\"]",
            "\"hidden\":[\"controls\",\"scenes\"]",
        );
        assert!(matches!(
            restored.restore_layout_json(&duplicate),
            Err(DockError::DuplicateDock(id)) if id == DockId::from("scenes")
        ));
    }

    #[test]
    fn missing_monitor_clamps_floating_window_to_primary_work_area() {
        let mut controller = controller();
        controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(4_000.0, -500.0, 2_000.0, 1_500.0),
            monitor: Some("gone".into()),
        });
        let changed = controller
            .clamp_floating_bounds(&BTreeMap::new(), DockBounds::new(0.0, 0.0, 1280.0, 900.0));
        assert!(changed);
        let floating = &controller.layout().floating[0];
        assert_eq!(floating.monitor, None);
        assert_eq!(floating.bounds, DockBounds::new(0.0, 0.0, 1280.0, 900.0));
    }

    #[test]
    fn locking_blocks_layout_mutations_but_keeps_tab_activation() {
        let mut controller = controller();
        controller.update(DockAction::SetLocked(true));
        assert!(
            !controller
                .update(DockAction::Hide("sources".into()))
                .changed
        );
        assert!(
            controller
                .update(DockAction::ActivateTab("sources".into()))
                .changed
        );
    }

    #[test]
    fn resize_and_keyboard_adjustment_respect_registered_minimums() {
        let mut controller = controller();
        controller.update(DockAction::SurfaceResized {
            surface: DockSurfaceId(0),
            width: 1_000.0,
            height: 700.0,
        });
        controller.update(DockAction::ResizeSplit {
            surface: DockSurfaceId(0),
            path: Vec::new(),
            ratio: 0.0,
        });
        let (_, ratio) = split_at_path(&controller.layout().main, &[]).expect("root split");
        assert!(ratio >= 0.15);

        controller.update(DockAction::AdjustSplit {
            surface: DockSurfaceId(0),
            path: Vec::new(),
            steps: 1.0,
        });
        let (_, adjusted) =
            split_at_path(&controller.layout().main, &[]).expect("adjusted root split");
        assert!(adjusted > ratio);
    }

    #[test]
    fn resize_uses_local_split_extent_and_reenters_at_the_pointer() {
        let mut controller = controller();
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(40.0, 60.0, 1_000.0, 700.0),
        });

        controller.update(DockAction::ResizeStart {
            surface: DockSurfaceId(0),
            path: vec![1],
        });
        controller.update(DockAction::ResizeMove(Point::new(600.0, 300.0)));
        controller.update(DockAction::ResizeMove(Point::new(600.0, 1_300.0)));
        let (_, maximum) =
            split_at_path(&controller.layout().main, &[1]).expect("nested split maximum");
        assert!(maximum < 1.0);

        let local_extent = 700.0 - DIVIDER_HIT_SIZE;
        controller.update(DockAction::ResizeMove(Point::new(
            600.0,
            300.0 + local_extent * 0.1,
        )));
        let (_, reentered) =
            split_at_path(&controller.layout().main, &[1]).expect("nested split reentered");
        assert!((reentered - 0.85).abs() < 0.000_1);

        controller.update(DockAction::ResizeEnd);
        controller.update(DockAction::ResizeStart {
            surface: DockSurfaceId(0),
            path: Vec::new(),
        });
        controller.update(DockAction::ResizeMove(Point::new(300.0, 200.0)));
        controller.update(DockAction::ResizeMove(Point::new(-1_000.0, 200.0)));
        controller.update(DockAction::ResizeMove(Point::new(399.2, 200.0)));
        let (_, root_reentered) =
            split_at_path(&controller.layout().main, &[]).expect("root split reentered");
        assert!((root_reentered - 0.35).abs() < 0.000_1);
    }

    #[test]
    fn drag_hit_testing_redocks_across_host_surfaces() {
        let mut controller = controller();
        let opened = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(1_400.0, 0.0, 360.0, 280.0),
            monitor: None,
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("floating surface")
        };
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(0.0, 0.0, 1_000.0, 700.0),
        });
        controller.update(DockAction::DragStart("sources".into()));
        controller.update(DockAction::DragMove(Point::new(1_500.0, 80.0)));
        controller.update(DockAction::DragMove(Point::new(300.0, 120.0)));
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: DockSurfaceId(0),
                id: "editor".into(),
                zone: DockDropZone::Left,
            })
        );
        let update = controller.update(DockAction::DragEnd);
        assert_eq!(
            update.effects,
            vec![DockHostEffect::CloseFloating(floating.surface)]
        );
        assert!(controller.layout().floating.is_empty());
        assert!(controller.is_visible(&DockId::from("sources")));
    }

    #[cfg(feature = "hosted")]
    #[test]
    fn hosted_adapter_preserves_floating_identity_and_geometry() {
        let mut controller = controller();
        let dock_update = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(40.0, 50.0, 360.0, 280.0),
            monitor: None,
        });
        let surface = controller.layout().floating[0].surface;
        let hosted = hosted_dock_update(dock_update, "NanaUI Dock");
        let HostedWindowCommand::Open { id, settings } = &hosted.window_commands[0] else {
            panic!("hosted open command")
        };
        assert_eq!(*id, HostedWindowId::from(surface));
        assert_eq!(settings.initial_position, Some((40.0, 50.0)));
        assert_eq!(settings.initial_size, Size::new(360.0, 280.0));
        let restored = controller.open_hosted_windows("NanaUI Dock");
        assert_eq!(restored.window_commands.len(), 1);

        let geometry = HostedWindowGeometry {
            physical_position: Some((120, 160)),
            physical_size: Size::new(800, 600),
            logical_position: Some(Point::new(60.0, 80.0)),
            logical_size: Size::new(400.0, 300.0),
            scale_factor: 2.0,
            maximized: false,
        };
        let update = controller.update_hosted_window(HostedWindowEvent::Moved {
            id: HostedWindowId::from(surface),
            window_id: iced::window::Id::unique(),
            geometry,
        });
        assert!(update.changed);
        assert_eq!(
            controller.layout().floating[0].bounds,
            DockBounds::new(60.0, 80.0, 400.0, 300.0)
        );
        controller.update(DockAction::SurfaceResized {
            surface,
            width: 420.0,
            height: 320.0,
        });
        assert_eq!(
            controller.layout().floating[0].bounds,
            DockBounds::new(60.0, 80.0, 420.0, 320.0)
        );

        let close = controller.update_hosted_window(HostedWindowEvent::CloseRequested {
            id: HostedWindowId::from(surface),
            window_id: iced::window::Id::unique(),
        });
        assert_eq!(close.effects, vec![DockHostEffect::CloseFloating(surface)]);
    }
}
