use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::widget::{button, column, container, row, space, stack, text};
use iced::{Alignment, Element, Event, Length, Padding, Point, Rectangle, Size, Subscription};
use serde::{Deserialize, Serialize};

use crate::drag_handle::DragHandle;
use crate::geometry::TITLE_BAR_HEIGHT as WINDOW_TITLE_BAR_HEIGHT;
use crate::resize_drag::{ResizeAxis, ResizeDrag};
use crate::shell::{
    window_chrome_controls, window_chrome_drag_start_area, window_chrome_drag_tracker,
};
use crate::theme::{ThemeTokens, ui_font};
use crate::widgets::{ButtonKind, CardKind, button_style, card_style};
use crate::window_chrome::{WindowChromeEvent, WindowChromeState};
#[cfg(feature = "hosted")]
use crate::{
    HostedProgramUpdate, HostedTitleBarMode, HostedWindowCommand, HostedWindowEvent,
    HostedWindowId, HostedWindowSettings,
};

const DOCK_LAYOUT_VERSION: u8 = 1;
const DIVIDER_HIT_SIZE: f32 = 8.0;
const TITLE_BAR_HEIGHT: f32 = 28.0;
const MIN_SPLIT_RATIO: f32 = 0.05;
const MAX_SPLIT_RATIO: f32 = 0.95;
const DRAG_INSERT_HOVER_DELAY: iced::time::Duration = iced::time::Duration::from_millis(80);
const DRAG_CARD_WIDTH: f32 = 280.0;
const DRAG_CARD_HEIGHT: f32 = 180.0;
const DRAG_CARD_OFFSET: f32 = 12.0;

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

/// Visual treatment for Dock chrome around application-owned content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockChromeStyle {
    #[default]
    Segmented,
    Borderless,
    Card,
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
    SurfaceLayout {
        surface: DockSurfaceId,
        bounds: DockBounds,
    },
    DragStart {
        surface: DockSurfaceId,
        id: DockId,
    },
    /// The source surface is recorded by [`DockAction::DragStart`]. During a
    /// drag, this is the surface that currently owns the pointer event, which
    /// may be a different floating surface.
    DragMove {
        surface: DockSurfaceId,
        position: Point,
    },
    /// Ends the active drag from the surface that currently owns the pointer.
    DragEnd {
        surface: DockSurfaceId,
    },
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
    MoveFloating {
        surface: DockSurfaceId,
        bounds: DockBounds,
    },
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
    InvalidFloatingSurface(DockSurfaceId),
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
    surface: DockSurfaceId,
    id: DockId,
    start: Option<Point>,
    position: Option<Point>,
    moved: bool,
    pending_target: Option<(DockDropTarget, iced::time::Instant)>,
    target: Option<DockDropTarget>,
    transient_surface: Option<DockSurfaceId>,
    transient_ready: bool,
    original_bounds: Option<DockBounds>,
    bounds: Option<DockBounds>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DockViewItem {
    Existing(DockId),
    Placeholder(DockId),
}

impl DockViewItem {
    fn id(&self) -> &DockId {
        match self {
            Self::Existing(id) | Self::Placeholder(id) => id,
        }
    }

    fn is_placeholder(&self) -> bool {
        matches!(self, Self::Placeholder(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum DockViewNode {
    Item {
        item: DockViewItem,
    },
    Tabs {
        tabs: Vec<DockViewItem>,
        active: DockViewItem,
    },
    Split {
        axis: DockAxis,
        ratio: f32,
        first: Box<DockViewNode>,
        second: Box<DockViewNode>,
    },
}

impl From<&DockNode> for DockViewNode {
    fn from(node: &DockNode) -> Self {
        match node {
            DockNode::Item { id } => Self::Item {
                item: DockViewItem::Existing(id.clone()),
            },
            DockNode::Tabs { tabs, active } => Self::Tabs {
                tabs: tabs.iter().cloned().map(DockViewItem::Existing).collect(),
                active: DockViewItem::Existing(active.clone()),
            },
            DockNode::Split {
                axis,
                ratio,
                first,
                second,
            } => Self::Split {
                axis: *axis,
                ratio: *ratio,
                first: Box::new(Self::from(first.as_ref())),
                second: Box::new(Self::from(second.as_ref())),
            },
        }
    }
}

impl DockViewNode {
    fn contains(&self, id: &DockId) -> bool {
        match self {
            Self::Item { item } => item.id() == id,
            Self::Tabs { tabs, .. } => tabs.iter().any(|item| item.id() == id),
            Self::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DockSurfaceGeometry {
    window: DockBounds,
    layout: Option<DockBounds>,
}

impl DockSurfaceGeometry {
    const fn new(window: DockBounds) -> Self {
        Self {
            window,
            layout: None,
        }
    }

    fn layout(self) -> DockBounds {
        self.layout.unwrap_or_else(|| self.default_layout())
    }

    fn global_layout(self) -> DockBounds {
        let layout = self.layout();
        DockBounds::new(
            self.window.x + layout.x,
            self.window.y + layout.y,
            layout.width,
            layout.height,
        )
    }

    fn local_to_global(self, position: Point) -> Point {
        Point::new(self.window.x + position.x, self.window.y + position.y)
    }

    fn set_window(&mut self, window: DockBounds) {
        if self.layout == Some(self.default_layout()) {
            self.layout = None;
        }
        self.window = window;
    }

    fn set_layout(&mut self, layout: DockBounds) {
        self.layout = Some(layout);
    }

    fn default_layout(self) -> DockBounds {
        DockBounds::new(0.0, 0.0, self.window.width, self.window.height)
    }
}

/// Owns a validated dock layout without owning native windows or GPU resources.
#[derive(Debug, Clone)]
pub struct DockController {
    center: DockId,
    specs: BTreeMap<DockId, DockItemSpec>,
    default_layout: DockLayout,
    layout: DockLayout,
    next_surface: u64,
    surface_geometry: BTreeMap<DockSurfaceId, DockSurfaceGeometry>,
    active_resize: Option<ActiveResize>,
    focused_split: Option<(DockSurfaceId, Vec<usize>, DockAxis)>,
    active_drag: Option<ActiveDrag>,
    chrome_style: DockChromeStyle,
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
            surface_geometry: BTreeMap::from([(
                DockSurfaceId(0),
                DockSurfaceGeometry::new(DockBounds::new(0.0, 0.0, 1280.0, 800.0)),
            )]),
            active_resize: None,
            focused_split: None,
            active_drag: None,
            chrome_style: DockChromeStyle::default(),
        })
    }

    pub fn layout(&self) -> &DockLayout {
        &self.layout
    }

    pub fn set_chrome_style(&mut self, chrome_style: DockChromeStyle) {
        self.chrome_style = chrome_style;
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

    fn drop_highlight_target(&self) -> Option<&DockDropTarget> {
        self.active_drag.as_ref().and_then(|drag| {
            drag.pending_target
                .as_ref()
                .map(|(target, _)| target)
                .or(drag.target.as_ref())
        })
    }

    pub fn is_drag_animation_active(&self) -> bool {
        false
    }

    /// Returns whether the host must keep requesting frames for the drag preview.
    ///
    /// A stationary drag only needs frames while a candidate is waiting for the
    /// insertion dwell. Once the target is settled, pointer events remain
    /// responsible for redraws.
    pub fn is_drag_frame_needed(&self) -> bool {
        self.active_drag
            .as_ref()
            .is_some_and(|drag| drag.pending_target.is_some())
    }

    #[cfg(test)]
    fn preview_root(&self) -> Option<DockViewNode> {
        self.preview_root_for(DockSurfaceId(0))
    }

    fn preview_root_for(&self, surface: DockSurfaceId) -> Option<DockViewNode> {
        let drag = self.active_drag.as_ref()?;
        let mut root = DockViewNode::from(self.surface_root(surface)?);
        if !drag.moved {
            return Some(root);
        }
        if drag.surface == surface && root.contains(&drag.id) {
            root = remove_view_node(root, &drag.id)?;
        }
        if let Some(target) = drag
            .target
            .as_ref()
            .filter(|target| target.surface == surface)
        {
            let placeholder = DockViewItem::Placeholder(drag.id.clone());
            insert_view_node(&mut root, &target.id, placeholder, target.zone);
        }
        Some(root)
    }

    fn settle_drag_target(&self, drag: &mut ActiveDrag, now: iced::time::Instant) {
        let Some((candidate, ready_at)) = drag.pending_target.as_ref() else {
            return;
        };
        if now < *ready_at {
            return;
        }
        let candidate = candidate.clone();
        drag.pending_target = None;
        drag.target = Some(candidate);
    }

    /// Provides arrow-key adjustment after a divider has been clicked or dragged.
    pub fn subscription(&self) -> Subscription<DockAction> {
        let mut subscriptions = Vec::new();
        match self.focused_split.as_ref().map(|focused| focused.2) {
            Some(DockAxis::Horizontal) => {
                subscriptions.push(iced::event::listen_with(dock_horizontal_key_event))
            }
            Some(DockAxis::Vertical) => {
                subscriptions.push(iced::event::listen_with(dock_vertical_key_event))
            }
            None => {}
        }
        if self.is_drag_frame_needed() {
            subscriptions.push(iced::window::frames().map(|_| DockAction::Hover(false)));
        }
        Subscription::batch(subscriptions)
    }

    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.layout)
    }

    pub fn restore_layout_json(&mut self, value: &str) -> Result<Vec<DockHostEffect>, DockError> {
        let restored: DockLayout = serde_json::from_str(value)
            .map_err(|error| DockError::InvalidJson(error.to_string()))?;
        let restored = reconcile_layout(restored, &self.default_layout, &self.specs, &self.center)?;
        let mut effects = surface_diff(&self.layout, &restored);
        let cleanup = self.cancel_drag();
        for effect in cleanup.effects {
            if let DockHostEffect::MoveFloating { surface, .. } = effect
                && effects.iter().any(|effect| {
                    matches!(effect, DockHostEffect::CloseFloating(closed) if *closed == surface)
                })
            {
                continue;
            }
            effects.push(effect);
        }
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
        self.retain_active_surface_geometry();
        for floating in &self.layout.floating {
            self.surface_geometry
                .entry(floating.surface)
                .or_insert(DockSurfaceGeometry::new(floating.bounds));
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
                    .surface_geometry
                    .get(&surface)
                    .map(|geometry| geometry.window)
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
                let bounds = DockBounds::new(
                    position.x,
                    position.y,
                    geometry.logical_size.width,
                    geometry.logical_size.height,
                );
                self.surface_geometry
                    .entry(surface)
                    .or_insert(DockSurfaceGeometry::new(bounds))
                    .set_window(bounds);
                if let Some(drag) = self.active_drag.as_mut()
                    && drag.transient_surface == Some(surface)
                {
                    drag.transient_ready = true;
                    drag.bounds = Some(bounds);
                    return DockUpdate::default();
                }
                self.update(DockAction::SurfaceGeometry { surface, bounds })
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
        self.open_hosted_windows_with_title_bar(title, HostedTitleBarMode::Native)
    }

    /// Reopens every floating surface with an explicit host title bar mode.
    #[cfg(feature = "hosted")]
    pub fn open_hosted_windows_with_title_bar(
        &self,
        title: impl Into<String>,
        title_bar_mode: HostedTitleBarMode,
    ) -> HostedProgramUpdate {
        hosted_dock_update_with_title_bar(
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
            title_bar_mode,
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
            self.surface_geometry
                .entry(floating.surface)
                .or_insert(DockSurfaceGeometry::new(bounds))
                .set_window(bounds);
        }
        changed
    }

    pub fn update(&mut self, action: DockAction) -> DockUpdate {
        self.update_at(action, iced::time::Instant::now())
    }

    fn update_at(&mut self, action: DockAction, now: iced::time::Instant) -> DockUpdate {
        if self.layout.locked
            && !matches!(
                action,
                DockAction::SetLocked(_)
                    | DockAction::Focus(_)
                    | DockAction::ActivateTab(_)
                    | DockAction::SurfaceResized { .. }
                    | DockAction::SurfaceGeometry { .. }
                    | DockAction::SurfaceLayout { .. }
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
                self.update_at(
                    DockAction::AdjustSplit {
                        surface,
                        path,
                        steps,
                    },
                    now,
                )
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
                let layout = DockBounds::new(0.0, 0.0, size.0, size.1);
                let is_drag_preview = self.is_drag_preview_surface(surface);
                let geometry = self
                    .surface_geometry
                    .entry(surface)
                    .or_insert(DockSurfaceGeometry::new(layout));
                let mut changed = geometry.layout() != layout;
                geometry.set_layout(layout);
                if is_drag_preview {
                    return DockUpdate::default();
                }
                if let Some(floating) = self
                    .layout
                    .floating
                    .iter_mut()
                    .find(|floating| floating.surface == surface)
                {
                    changed |= floating.bounds.width != size.0 || floating.bounds.height != size.1;
                    floating.bounds.width = size.0;
                    floating.bounds.height = size.1;
                    geometry.set_window(floating.bounds);
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
                self.surface_geometry
                    .entry(surface)
                    .or_insert(DockSurfaceGeometry::new(bounds))
                    .set_window(bounds);
                if self.is_drag_preview_surface(surface) {
                    if let Some(drag) = self.active_drag.as_mut() {
                        drag.bounds = Some(bounds);
                    }
                    return DockUpdate::default();
                }
                let mut changed = false;
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
            DockAction::SurfaceLayout { surface, bounds } => {
                if !valid_bounds(bounds) {
                    return DockUpdate::default();
                }
                self.surface_geometry
                    .entry(surface)
                    .or_insert(DockSurfaceGeometry::new(bounds))
                    .set_layout(bounds);
                DockUpdate::default()
            }
            DockAction::DragStart { surface, id } => {
                if id == self.center
                    || !self.is_visible(&id)
                    || !self
                        .surface_root(surface)
                        .is_some_and(|root| root.contains(&id))
                {
                    return DockUpdate::default();
                }
                self.active_drag = Some(ActiveDrag {
                    surface,
                    id,
                    start: None,
                    position: None,
                    moved: false,
                    pending_target: None,
                    target: None,
                    transient_surface: None,
                    transient_ready: false,
                    original_bounds: None,
                    bounds: None,
                });
                DockUpdate::default()
            }
            DockAction::DragMove { surface, position } => {
                let Some(mut drag) = self.active_drag.take() else {
                    return DockUpdate::default();
                };
                let position = self.local_to_global(surface, position);
                let start = *drag.start.get_or_insert(position);
                drag.moved |= (position.x - start.x)
                    .abs()
                    .max((position.y - start.y).abs())
                    >= 4.0;
                drag.position = Some(position);
                let next_candidate = drag
                    .moved
                    .then(|| self.drop_target_at(&drag.id, position, drag.surface, surface))
                    .flatten();
                let current_candidate = drag
                    .pending_target
                    .as_ref()
                    .map(|(target, _)| target)
                    .or(drag.target.as_ref());
                if next_candidate.as_ref() != current_candidate {
                    drag.pending_target =
                        next_candidate.map(|target| (target, now + DRAG_INSERT_HOVER_DELAY));
                    drag.target = None;
                }
                self.settle_drag_target(&mut drag, now);
                let mut effects = Vec::new();
                if drag.moved
                    && drag.transient_surface.is_none()
                    && let Some(effect) = self.begin_transient_drag(&mut drag, position)
                {
                    effects.push(effect);
                }
                if let Some(surface) = drag.transient_surface
                    && let Some(bounds) = self.drag_bounds(&drag, position)
                    && drag.bounds != Some(bounds)
                {
                    drag.bounds = Some(bounds);
                    self.surface_geometry
                        .entry(surface)
                        .or_insert(DockSurfaceGeometry::new(bounds))
                        .set_window(bounds);
                    if drag.transient_ready {
                        effects.push(DockHostEffect::MoveFloating { surface, bounds });
                    }
                }
                self.active_drag = Some(drag);
                DockUpdate {
                    changed: false,
                    effects,
                }
            }
            DockAction::DragEnd { surface: _ } => {
                let Some(drag) = self.active_drag.take() else {
                    return DockUpdate::default();
                };
                let mut drag = drag;
                self.settle_drag_target(&mut drag, now);
                if !drag.moved {
                    return DockUpdate {
                        changed: activate_tab_layout(&mut self.layout, &drag.id),
                        effects: Vec::new(),
                    };
                }
                if let Some(target) = drag.target {
                    let transient_surface = drag.transient_surface;
                    let mut update = self.dock(drag.id, target);
                    if let Some(surface) = transient_surface {
                        if !update.effects.iter().any(|effect| {
                            matches!(effect, DockHostEffect::CloseFloating(closed) if *closed == surface)
                        }) {
                            update.effects.push(DockHostEffect::CloseFloating(surface));
                        }
                        self.surface_geometry.remove(&surface);
                    }
                    return update;
                }
                self.promote_drag_to_floating(drag)
            }
            DockAction::CancelDrag => self.cancel_drag(),
            DockAction::Hover(_) => {
                let Some(mut drag) = self.active_drag.take() else {
                    return DockUpdate::default();
                };
                self.settle_drag_target(&mut drag, now);
                self.active_drag = Some(drag);
                DockUpdate::default()
            }
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
                let before = self.layout.clone();
                let active_drag = self.active_drag.take();
                let mut effects = surface_diff(&before, &self.default_layout);
                let changed = before != self.default_layout;
                self.layout = self.default_layout.clone();
                self.active_resize = None;
                self.focused_split = None;
                if let Some(drag) = active_drag
                    && let Some(surface) = drag.transient_surface
                {
                    if let Some(floating) = self
                        .layout
                        .floating
                        .iter()
                        .find(|floating| floating.surface == surface)
                    {
                        let bounds = floating.bounds;
                        self.surface_geometry
                            .entry(surface)
                            .or_insert(DockSurfaceGeometry::new(bounds))
                            .set_window(bounds);
                        if drag.bounds != Some(bounds) {
                            effects.push(DockHostEffect::MoveFloating { surface, bounds });
                        }
                    } else if !effects.iter().any(|effect| {
                        matches!(
                            effect,
                            DockHostEffect::CloseFloating(closed) if *closed == surface
                        )
                    }) {
                        effects.push(DockHostEffect::CloseFloating(surface));
                    }
                }
                self.surface_geometry
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
        self.surface_geometry
            .insert(floating.surface, DockSurfaceGeometry::new(bounds));
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
        if id == self.center
            || id == target.id
            || !self.specs.contains_key(&id)
            || (target.zone == DockDropZone::Tab && target.id == self.center)
            || !self
                .surface_root(target.surface)
                .is_some_and(|root| root.contains(&target.id))
        {
            return DockUpdate::default();
        }
        let before = self.layout.clone();
        let (removed, closed_surface) = remove_from_layout(&mut self.layout, &id);
        if !removed {
            return DockUpdate::default();
        }
        self.layout.hidden.retain(|hidden| hidden != &id);
        let node = DockNode::item(id);
        let inserted =
            self.surface_root_mut(target.surface)
                .is_some_and(|root| match target.zone {
                    DockDropZone::Tab => insert_tab(root, &target.id, node.clone()),
                    zone => insert_split(root, &target.id, node.clone(), zone),
                });
        if !inserted {
            self.layout = before;
            return DockUpdate::default();
        }
        DockUpdate {
            changed: true,
            effects: closed_surface
                .map(DockHostEffect::CloseFloating)
                .into_iter()
                .collect(),
        }
    }

    fn drag_card_bounds(position: Point) -> DockBounds {
        DockBounds::new(
            position.x + DRAG_CARD_OFFSET,
            position.y + DRAG_CARD_OFFSET,
            DRAG_CARD_WIDTH,
            DRAG_CARD_HEIGHT,
        )
    }

    fn begin_transient_drag(
        &mut self,
        drag: &mut ActiveDrag,
        position: Point,
    ) -> Option<DockHostEffect> {
        if drag.transient_surface.is_some() {
            return None;
        }
        if drag.surface == DockSurfaceId(0) {
            let surface = DockSurfaceId(self.next_surface);
            self.next_surface = self.next_surface.saturating_add(1);
            let bounds = Self::drag_card_bounds(position);
            drag.transient_surface = Some(surface);
            drag.transient_ready = false;
            drag.bounds = Some(bounds);
            self.surface_geometry
                .insert(surface, DockSurfaceGeometry::new(bounds));
            Some(DockHostEffect::OpenFloating(FloatingDock {
                surface,
                root: DockNode::item(drag.id.clone()),
                bounds,
                monitor: None,
            }))
        } else {
            let bounds = self.surface_window_bounds(drag.surface);
            drag.original_bounds = Some(bounds);
            drag.bounds = Some(bounds);
            let reuse_source = self
                .surface_root(drag.surface)
                .is_some_and(|root| matches!(root, DockNode::Item { id } if id == &drag.id));
            if reuse_source {
                drag.transient_surface = Some(drag.surface);
                drag.transient_ready = true;
                None
            } else {
                let surface = DockSurfaceId(self.next_surface);
                self.next_surface = self.next_surface.saturating_add(1);
                let monitor = self
                    .layout
                    .floating
                    .iter()
                    .find(|floating| floating.surface == drag.surface)
                    .and_then(|floating| floating.monitor.clone());
                drag.transient_surface = Some(surface);
                drag.transient_ready = false;
                self.surface_geometry
                    .insert(surface, DockSurfaceGeometry::new(bounds));
                Some(DockHostEffect::OpenFloating(FloatingDock {
                    surface,
                    root: DockNode::item(drag.id.clone()),
                    bounds,
                    monitor,
                }))
            }
        }
    }

    fn drag_bounds(&self, drag: &ActiveDrag, position: Point) -> Option<DockBounds> {
        let current = drag.bounds?;
        let bounds = if drag.surface == DockSurfaceId(0) {
            DockBounds::new(
                position.x + DRAG_CARD_OFFSET,
                position.y + DRAG_CARD_OFFSET,
                current.width,
                current.height,
            )
        } else {
            let original = drag.original_bounds?;
            let start = drag.start?;
            DockBounds::new(
                original.x + position.x - start.x,
                original.y + position.y - start.y,
                current.width,
                current.height,
            )
        };
        valid_bounds(bounds).then_some(bounds)
    }

    fn promote_drag_to_floating(&mut self, drag: ActiveDrag) -> DockUpdate {
        let position = drag.position;
        let Some(surface) = drag.transient_surface else {
            return position.map_or_else(DockUpdate::default, |position| {
                self.float(drag.id, Self::drag_card_bounds(position), None)
            });
        };
        let bounds = drag.bounds.or_else(|| position.map(Self::drag_card_bounds));
        let Some(bounds) = bounds.filter(|bounds| valid_bounds(*bounds)) else {
            return DockUpdate::default();
        };
        if drag.surface == DockSurfaceId(0) {
            let (removed, closed_surface) = remove_from_layout(&mut self.layout, &drag.id);
            if !removed {
                return DockUpdate::default();
            }
            self.layout.hidden.retain(|hidden| hidden != &drag.id);
            let floating = FloatingDock {
                surface,
                root: DockNode::item(drag.id),
                bounds,
                monitor: None,
            };
            self.layout.floating.push(floating);
            self.surface_geometry
                .insert(surface, DockSurfaceGeometry::new(bounds));
            DockUpdate {
                changed: true,
                effects: closed_surface
                    .map(DockHostEffect::CloseFloating)
                    .into_iter()
                    .collect(),
            }
        } else if drag.transient_surface == Some(drag.surface) {
            let Some(floating) = self
                .layout
                .floating
                .iter_mut()
                .find(|floating| floating.surface == surface)
            else {
                return DockUpdate::default();
            };
            let changed = floating.bounds != bounds;
            floating.bounds = bounds;
            self.surface_geometry
                .entry(surface)
                .or_insert(DockSurfaceGeometry::new(bounds))
                .set_window(bounds);
            DockUpdate {
                changed,
                effects: Vec::new(),
            }
        } else {
            let monitor = self
                .layout
                .floating
                .iter()
                .find(|floating| floating.surface == drag.surface)
                .and_then(|floating| floating.monitor.clone());
            let (removed, closed_surface) = remove_from_layout(&mut self.layout, &drag.id);
            if !removed {
                return DockUpdate::default();
            }
            self.layout.hidden.retain(|hidden| hidden != &drag.id);
            self.layout.floating.push(FloatingDock {
                surface,
                root: DockNode::item(drag.id),
                bounds,
                monitor,
            });
            self.surface_geometry
                .entry(surface)
                .or_insert(DockSurfaceGeometry::new(bounds))
                .set_window(bounds);
            DockUpdate {
                changed: true,
                effects: closed_surface
                    .map(DockHostEffect::CloseFloating)
                    .into_iter()
                    .collect(),
            }
        }
    }

    fn cancel_drag(&mut self) -> DockUpdate {
        let Some(drag) = self.active_drag.take() else {
            return DockUpdate::default();
        };
        let Some(surface) = drag.transient_surface else {
            return DockUpdate::default();
        };
        if drag.surface == DockSurfaceId(0) || drag.transient_surface != Some(drag.surface) {
            self.surface_geometry.remove(&surface);
            DockUpdate {
                changed: false,
                effects: vec![DockHostEffect::CloseFloating(surface)],
            }
        } else {
            let original = drag.original_bounds.or_else(|| {
                self.layout
                    .floating
                    .iter()
                    .find(|floating| floating.surface == surface)
                    .map(|floating| floating.bounds)
            });
            let Some(original) = original else {
                return DockUpdate::default();
            };
            self.surface_geometry
                .entry(surface)
                .or_insert(DockSurfaceGeometry::new(original))
                .set_window(original);
            let effects = (drag.bounds != Some(original))
                .then_some(DockHostEffect::MoveFloating {
                    surface,
                    bounds: original,
                })
                .into_iter()
                .collect();
            DockUpdate {
                changed: false,
                effects,
            }
        }
    }

    fn drag_floating(&self, surface: DockSurfaceId) -> Option<FloatingDock> {
        let drag = self.active_drag.as_ref()?;
        (drag.transient_surface == Some(surface)).then(|| FloatingDock {
            surface,
            root: DockNode::item(drag.id.clone()),
            bounds: drag
                .bounds
                .unwrap_or_else(|| self.surface_window_bounds(surface)),
            monitor: None,
        })
    }

    fn is_drag_preview_surface(&self, surface: DockSurfaceId) -> bool {
        self.active_drag
            .as_ref()
            .is_some_and(|drag| drag.transient_surface == Some(surface))
    }

    fn close_surface(&mut self, surface: DockSurfaceId) -> DockUpdate {
        let is_drag_surface = self.is_drag_preview_surface(surface);
        if is_drag_surface
            && !self
                .layout
                .floating
                .iter()
                .any(|floating| floating.surface == surface)
        {
            self.active_drag = None;
            self.surface_geometry.remove(&surface);
            return DockUpdate::default();
        }
        if is_drag_surface {
            self.active_drag = None;
        }
        let Some(index) = self
            .layout
            .floating
            .iter()
            .position(|floating| floating.surface == surface)
        else {
            return DockUpdate::default();
        };
        let floating = self.layout.floating.remove(index);
        self.surface_geometry.remove(&surface);
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

    fn surface_root_mut(&mut self, surface: DockSurfaceId) -> Option<&mut DockNode> {
        if surface == DockSurfaceId(0) {
            Some(&mut self.layout.main)
        } else {
            self.layout
                .floating
                .iter_mut()
                .find(|floating| floating.surface == surface)
                .map(|floating| &mut floating.root)
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
        self.surface_root_mut(surface)
            .is_some_and(|root| set_split_ratio(root, path, ratio))
    }

    fn split_geometry(
        &self,
        surface: DockSurfaceId,
        path: &[usize],
    ) -> Option<(DockAxis, f32, f32)> {
        let root = self.surface_root(surface)?;
        let bounds = self.surface_layout_bounds(surface);
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

    fn drop_target_at(
        &self,
        dragged: &DockId,
        position: Point,
        drag_surface: DockSurfaceId,
        hover_surface: DockSurfaceId,
    ) -> Option<DockDropTarget> {
        let surface = hover_surface;
        let root = self.surface_root(surface)?;
        let bounds = self.global_layout_bounds(surface);
        if !bounds_contains(bounds, position) {
            return None;
        }
        let mut view_root = DockViewNode::from(root);
        if drag_surface == surface && view_root.contains(dragged) {
            view_root = remove_view_node(view_root, dragged)?;
        }
        let mut targets = Vec::new();
        collect_view_drop_targets(&view_root, bounds, &mut targets);
        let (id, bounds) = targets
            .into_iter()
            .find(|(id, bounds)| id != dragged && bounds_contains(*bounds, position))?;
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
            return None;
        } else {
            DockDropZone::Tab
        };
        Some(DockDropTarget { surface, id, zone })
    }

    fn retain_active_surface_geometry(&mut self) {
        self.surface_geometry.retain(|surface, _| {
            *surface == DockSurfaceId(0)
                || self
                    .layout
                    .floating
                    .iter()
                    .any(|floating| floating.surface == *surface)
        });
    }

    fn surface_window_bounds(&self, surface: DockSurfaceId) -> DockBounds {
        self.surface_geometry
            .get(&surface)
            .map(|geometry| geometry.window)
            .or_else(|| {
                self.layout
                    .floating
                    .iter()
                    .find(|floating| floating.surface == surface)
                    .map(|floating| floating.bounds)
            })
            .unwrap_or(DockBounds::new(0.0, 0.0, 1280.0, 800.0))
    }

    fn surface_layout_bounds(&self, surface: DockSurfaceId) -> DockBounds {
        self.surface_geometry
            .get(&surface)
            .copied()
            .map(DockSurfaceGeometry::layout)
            .unwrap_or_else(|| {
                DockSurfaceGeometry::new(self.surface_window_bounds(surface)).layout()
            })
    }

    fn global_layout_bounds(&self, surface: DockSurfaceId) -> DockBounds {
        self.surface_geometry
            .get(&surface)
            .copied()
            .unwrap_or_else(|| DockSurfaceGeometry::new(self.surface_window_bounds(surface)))
            .global_layout()
    }

    fn local_to_global(&self, surface: DockSurfaceId, position: Point) -> Point {
        self.surface_geometry
            .get(&surface)
            .copied()
            .unwrap_or_else(|| DockSurfaceGeometry::new(self.surface_window_bounds(surface)))
            .local_to_global(position)
    }
}

/// Converts Dock window effects into commands understood by [`crate::run_hosted`].
#[cfg(feature = "hosted")]
pub fn hosted_dock_update(update: DockUpdate, title: impl Into<String>) -> HostedProgramUpdate {
    hosted_dock_update_with_title_bar(update, title, HostedTitleBarMode::Native)
}

/// Converts Dock window effects with an explicit host title bar mode.
#[cfg(feature = "hosted")]
pub fn hosted_dock_update_with_title_bar(
    update: DockUpdate,
    title: impl Into<String>,
    title_bar_mode: HostedTitleBarMode,
) -> HostedProgramUpdate {
    let title = title.into();
    let commands = update.effects.into_iter().map(|effect| match effect {
        DockHostEffect::OpenFloating(floating) => HostedWindowCommand::Open {
            id: HostedWindowId::from(floating.surface),
            settings: HostedWindowSettings::new(title.clone())
                .tool_window()
                .title_bar_mode(title_bar_mode)
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
        DockHostEffect::MoveFloating { surface, bounds } => HostedWindowCommand::Move {
            id: HostedWindowId::from(surface),
            position: Point::new(bounds.x, bounds.y),
        },
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

#[derive(Debug, Clone, Copy)]
enum DockSurfacePointer {
    Move(Point),
    End,
}

struct DockSurface<'a, Message> {
    content: Element<'a, Message>,
    on_geometry: Rc<dyn Fn(DockBounds) -> Message + 'a>,
    on_pointer: Option<Rc<dyn Fn(DockSurfacePointer) -> Option<Message> + 'a>>,
}

impl<'a, Message> DockSurface<'a, Message> {
    fn new(
        content: impl Into<Element<'a, Message>>,
        on_geometry: impl Fn(DockBounds) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            on_geometry: Rc::new(on_geometry),
            on_pointer: None,
        }
    }

    fn with_pointer(
        mut self,
        on_pointer: impl Fn(DockSurfacePointer) -> Option<Message> + 'a,
    ) -> Self {
        self.on_pointer = Some(Rc::new(on_pointer));
        self
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
        if let Some(on_pointer) = &self.on_pointer {
            let signal = match event {
                Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    Some(DockSurfacePointer::Move(*position))
                }
                Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                    Some(DockSurfacePointer::End)
                }
                Event::Touch(iced::touch::Event::FingerMoved { position, .. }) => {
                    Some(DockSurfacePointer::Move(*position))
                }
                Event::Touch(
                    iced::touch::Event::FingerLifted { .. } | iced::touch::Event::FingerLost { .. },
                ) => Some(DockSurfacePointer::End),
                _ => None,
            };
            if let Some(message) = signal.and_then(|signal| on_pointer(signal)) {
                shell.publish(message);
                shell.capture_event();
                return;
            }
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

fn dock_surface_pointer_handler<'a, Message>(
    controller: &DockController,
    surface: DockSurfaceId,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
) -> Option<Rc<dyn Fn(DockSurfacePointer) -> Option<Message> + 'a>>
where
    Message: 'a,
{
    let drag = controller.active_drag.as_ref()?;
    if drag.transient_surface.is_none() && drag.surface == surface {
        return None;
    }
    Some(Rc::new(move |pointer| match pointer {
        DockSurfacePointer::Move(position) => {
            Some(on_action(DockAction::DragMove { surface, position }))
        }
        DockSurfacePointer::End => Some(on_action(DockAction::DragEnd { surface })),
    }))
}

fn dock_surface_view<'a, Message>(
    controller: &DockController,
    surface: DockSurfaceId,
    content: impl Into<Element<'a, Message>>,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let surface_id = surface;
    let surface = DockSurface::new(content, move |bounds| {
        on_action(DockAction::SurfaceLayout {
            surface: surface_id,
            bounds,
        })
    });
    let surface =
        if let Some(pointer) = dock_surface_pointer_handler(controller, surface_id, on_action) {
            surface.with_pointer(move |event| pointer(event))
        } else {
            surface
        };
    Element::new(surface)
}

/// Renders one dock surface. Floating surfaces use the same controller and a different root.
pub fn dock_workspace<'a, Message>(
    controller: &DockController,
    surface: DockSurfaceId,
    mut contents: DockContents<'a, Message>,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    theme: impl Into<ThemeTokens>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let tokens = theme.into();
    let root = controller
        .preview_root_for(surface)
        .or_else(|| controller.surface_root(surface).map(DockViewNode::from));
    let content = root.map_or_else(
        || {
            container(space())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        },
        |root| {
            dock_node_view(
                &root,
                surface,
                Vec::new(),
                controller,
                &mut contents,
                on_action,
                tokens,
            )
        },
    );
    let content = if surface == DockSurfaceId(0) {
        dock_fallback_drag_card(controller, content, tokens)
    } else {
        content
    };
    dock_surface_view(controller, surface, content, on_action)
}

/// Renders a floating Dock surface with the custom window title bar.
pub fn dock_window_workspace<'a, Message>(
    controller: &DockController,
    surface: DockSurfaceId,
    mut contents: DockContents<'a, Message>,
    window_chrome: &WindowChromeState,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    on_window_event: impl Fn(WindowChromeEvent) -> Message + 'a,
    theme: impl Into<ThemeTokens>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let tokens = theme.into();
    let on_window_event = Rc::new(on_window_event);
    let root = controller
        .drag_floating(surface)
        .map(|floating| DockViewNode::from(&floating.root))
        .or_else(|| controller.preview_root_for(surface))
        .or_else(|| controller.surface_root(surface).map(DockViewNode::from));
    let content = root.map_or_else(
        || {
            container(space())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        },
        |root| {
            dock_window_root_view(
                &root,
                surface,
                controller,
                &mut contents,
                window_chrome,
                on_action,
                on_window_event,
                tokens,
            )
        },
    );
    let content = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(tokens.colors.background)
                .color(tokens.colors.text)
        });
    dock_surface_view(controller, surface, content, on_action)
}

fn dock_fallback_drag_card<'a, Message>(
    controller: &DockController,
    content: Element<'a, Message>,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let Some(drag) = controller.active_drag.as_ref() else {
        return content;
    };
    if drag.surface != DockSurfaceId(0) || drag.transient_surface.is_none() || drag.transient_ready
    {
        return content;
    }
    let Some(position) = drag.position else {
        return content;
    };
    let bounds = controller.global_layout_bounds(DockSurfaceId(0));
    let x = (position.x - bounds.x + DRAG_CARD_OFFSET).max(0.0);
    let y = (position.y - bounds.y + DRAG_CARD_OFFSET).max(0.0);
    let card = container(dock_drag_preview_view(&drag.id, controller, tokens))
        .width(Length::Fixed(DRAG_CARD_WIDTH))
        .height(Length::Fixed(DRAG_CARD_HEIGHT));
    let follower = container(
        column![
            space().height(Length::Fixed(y)),
            row![space().width(Length::Fixed(x)), card],
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill);
    stack![content, follower]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn dock_window_root_view<'a, Message>(
    node: &DockViewNode,
    surface: DockSurfaceId,
    controller: &DockController,
    contents: &mut DockContents<'a, Message>,
    window_chrome: &WindowChromeState,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    on_window_event: Rc<dyn Fn(WindowChromeEvent) -> Message + 'a>,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    match node {
        DockViewNode::Item {
            item: DockViewItem::Existing(id),
        } => dock_window_item_view(
            id,
            surface,
            controller,
            contents,
            window_chrome,
            on_action,
            on_window_event,
            tokens,
        ),
        DockViewNode::Item {
            item: DockViewItem::Placeholder(id),
        } => dock_drag_preview_view(id, controller, tokens),
        _ => {
            let title_bar = dock_window_title_bar(
                controller,
                surface,
                dock_view_active_id(node),
                window_chrome,
                on_action,
                on_window_event,
                tokens,
            );
            let body = dock_node_view(
                node,
                surface,
                Vec::new(),
                controller,
                contents,
                on_action,
                tokens,
            );
            column![title_bar, body]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    }
}

fn dock_view_active_id(node: &DockViewNode) -> Option<&DockId> {
    match node {
        DockViewNode::Item { item } => Some(item.id()),
        DockViewNode::Tabs { active, .. } => Some(active.id()),
        DockViewNode::Split { first, .. } => dock_view_active_id(first),
    }
}

fn dock_window_title_bar<'a, Message>(
    controller: &DockController,
    surface: DockSurfaceId,
    title_id: Option<&DockId>,
    window_chrome: &WindowChromeState,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    on_window_event: Rc<dyn Fn(WindowChromeEvent) -> Message + 'a>,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let title = container(
        text(title_id.map_or_else(String::new, |id| dock_item_title(controller, id)))
            .size(11)
            .font(ui_font(iced::font::Weight::Semibold)),
    )
    .height(Length::Fill)
    .padding(Padding {
        top: 0.0,
        right: 8.0,
        bottom: 0.0,
        left: 0.0,
    })
    .center_y(Length::Fill);
    let drag_region = row![title, space().width(Length::Fill).height(Length::Fill)]
        .height(Length::Fill)
        .align_y(Alignment::Center);
    let drag_region: Element<'a, Message> = if controller.layout.locked {
        window_chrome_drag_start_area(drag_region, &on_window_event)
    } else if let Some(id) = title_id {
        dock_drag_handle(
            drag_region,
            id,
            surface,
            on_action,
            DockAction::Focus(id.clone()),
        )
    } else {
        drag_region.into()
    };
    let chrome = window_chrome.chrome();
    let controls = window_chrome_controls(
        chrome,
        window_chrome.is_maximized(),
        tokens,
        &on_window_event,
    );
    let controls = container(controls)
        .height(Length::Fill)
        .align_y(iced::alignment::Vertical::Center)
        .padding(Padding {
            top: 0.0,
            right: 6.0 + chrome.trailing_inset,
            bottom: 0.0,
            left: 6.0,
        });
    let title_bar = container(
        row![
            space().width(Length::Fixed(6.0 + chrome.leading_inset)),
            drag_region,
            controls,
        ]
        .height(Length::Fill)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(WINDOW_TITLE_BAR_HEIGHT))
    .clip(true)
    .style(move |_theme| {
        iced::widget::container::Style::default()
            .background(tokens.colors.surface)
            .color(tokens.colors.text)
    });
    if controller.layout.locked {
        window_chrome_drag_tracker(title_bar, on_window_event)
    } else {
        title_bar.into()
    }
}

fn dock_window_item_view<'a, Message>(
    id: &DockId,
    surface: DockSurfaceId,
    controller: &DockController,
    contents: &mut DockContents<'a, Message>,
    window_chrome: &WindowChromeState,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    on_window_event: Rc<dyn Fn(WindowChromeEvent) -> Message + 'a>,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    if controller.is_drag_preview_surface(surface) {
        return dock_drag_preview_view(id, controller, tokens);
    }
    let title_bar = dock_window_title_bar(
        controller,
        surface,
        Some(id),
        window_chrome,
        on_action,
        on_window_event,
        tokens,
    );
    let body = dock_item_body(id, contents, tokens, controller.chrome_style);
    let window = column![title_bar, body]
        .width(Length::Fill)
        .height(Length::Fill);
    if controller.chrome_style == DockChromeStyle::Card {
        dock_card_shell(window, tokens)
    } else {
        window.into()
    }
}

fn dock_node_view<'a, Message>(
    node: &DockViewNode,
    surface: DockSurfaceId,
    path: Vec<usize>,
    controller: &DockController,
    contents: &mut DockContents<'a, Message>,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let highlight = controller
        .drop_highlight_target()
        .filter(|target| target.surface == surface);
    match node {
        DockViewNode::Item { item } => {
            let view = dock_view_item_view(
                item, surface, false, controller, contents, on_action, tokens,
            );
            if is_drop_highlighted(highlight, item.id()) {
                dock_insert_highlight(view, tokens)
            } else {
                view
            }
        }
        DockViewNode::Tabs { tabs, active } => {
            let chrome_style = controller.chrome_style;
            let tab_bar = tabs.iter().fold(
                row![].height(Length::Fixed(TITLE_BAR_HEIGHT)),
                |tabs_row, item| {
                    let id = item.id();
                    let title = dock_item_title(controller, id);
                    let placeholder = item.is_placeholder();
                    let highlighted = is_drop_highlighted(highlight, id);
                    let active_tab = item == active;
                    let label = text(title)
                        .size(11)
                        .font(ui_font(iced::font::Weight::Medium));
                    let tab = container(label)
                        .center_y(Length::Fill)
                        .padding([0.0, 10.0])
                        .style(move |_theme| {
                            let background = if placeholder || highlighted {
                                tokens.colors.accent_soft
                            } else if active_tab {
                                tokens.colors.active
                            } else if chrome_style == DockChromeStyle::Card {
                                iced::Color::TRANSPARENT
                            } else {
                                tokens.colors.surface
                            };
                            iced::widget::container::Style::default()
                                .background(background)
                                .border(if placeholder || highlighted {
                                    dock_insert_preview_border(tokens)
                                } else {
                                    dock_tab_border(tokens, chrome_style)
                                })
                        });
                    if placeholder {
                        tabs_row.push(
                            tab.height(Length::Fixed(TITLE_BAR_HEIGHT))
                                .width(Length::Shrink),
                        )
                    } else if controller.layout.locked {
                        tabs_row.push(
                            button(tab)
                                .height(Length::Fixed(TITLE_BAR_HEIGHT))
                                .padding(0)
                                .on_press(on_action(DockAction::ActivateTab(id.clone())))
                                .style(button_style(tokens, ButtonKind::Text)),
                        )
                    } else {
                        tabs_row.push(dock_drag_handle(
                            tab.height(Length::Fixed(TITLE_BAR_HEIGHT))
                                .width(Length::Shrink),
                            id,
                            surface,
                            on_action,
                            DockAction::ActivateTab(id.clone()),
                        ))
                    }
                },
            );
            let active_item = dock_view_item_view(
                active, surface, true, controller, contents, on_action, tokens,
            );
            if chrome_style == DockChromeStyle::Card {
                let tab_bar = dock_card_title_bar(tab_bar, tokens);
                dock_card_shell(column![tab_bar, active_item].height(Length::Fill), tokens)
            } else {
                let tab_bar = container(tab_bar)
                    .width(Length::Fill)
                    .height(Length::Fixed(TITLE_BAR_HEIGHT))
                    .style(move |_theme| {
                        iced::widget::container::Style::default()
                            .background(tokens.colors.surface)
                            .border(dock_chrome_border(tokens, chrome_style))
                    });
                column![tab_bar, active_item]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        }
        DockViewNode::Split {
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
            let divider_color = dock_chrome_color(tokens, controller.chrome_style);
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
                    iced::widget::container::Style::default().background(divider_color)
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

fn dock_view_item_view<'a, Message>(
    item: &DockViewItem,
    surface: DockSurfaceId,
    tabs_own_title: bool,
    controller: &DockController,
    contents: &mut DockContents<'a, Message>,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    match item {
        DockViewItem::Existing(id) => dock_item_view(
            id,
            surface,
            tabs_own_title,
            controller,
            contents,
            on_action,
            tokens,
        ),
        DockViewItem::Placeholder(id) => {
            dock_placeholder_view(id, tabs_own_title, controller, tokens)
        }
    }
}

fn dock_item_view<'a, Message>(
    id: &DockId,
    surface: DockSurfaceId,
    tabs_own_title: bool,
    controller: &DockController,
    contents: &mut DockContents<'a, Message>,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let chrome_style = controller.chrome_style;
    let spec = controller.item(id);
    let title = dock_item_title(controller, id);
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
        dock_drag_handle(title, id, surface, on_action, DockAction::Focus(id.clone()))
    };
    let mut title_bar = row![title].spacing(4).align_y(Alignment::Center);
    if !controller.layout.locked && id != &controller.center && surface == DockSurfaceId(0) {
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
    let content_chrome_style = if chrome_style == DockChromeStyle::Card && id == &controller.center
    {
        DockChromeStyle::Borderless
    } else {
        chrome_style
    };
    let content = dock_item_body(id, contents, tokens, content_chrome_style);
    if tabs_own_title || id == &controller.center {
        content
    } else if chrome_style == DockChromeStyle::Card {
        dock_card_shell(
            column![dock_card_title_bar(title_bar, tokens), content].height(Length::Fill),
            tokens,
        )
    } else {
        column![
            container(title_bar)
                .width(Length::Fill)
                .height(Length::Fixed(TITLE_BAR_HEIGHT))
                .padding([0.0, 6.0])
                .style(move |_theme| iced::widget::container::Style::default()
                    .background(tokens.colors.surface)
                    .border(dock_chrome_border(tokens, chrome_style))),
            content,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

fn dock_item_title(controller: &DockController, id: &DockId) -> String {
    controller
        .item(id)
        .map_or_else(|| id.as_str().to_owned(), |spec| spec.title.clone())
}

fn is_drop_highlighted(highlight: Option<&DockDropTarget>, id: &DockId) -> bool {
    highlight.is_some_and(|target| target.id == *id)
}

fn dock_insert_highlight<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let content: Element<'a, Message> = content.into();
    let overlay = container(space())
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(tokens.colors.accent_soft)
                .border(dock_insert_preview_border(tokens))
        });
    stack![content, overlay]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn dock_placeholder_view<'a, Message>(
    id: &DockId,
    tabs_own_title: bool,
    controller: &DockController,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let body = dock_placeholder_body(tokens, controller.chrome_style);
    if tabs_own_title {
        return body;
    }

    let title = container(
        text(dock_item_title(controller, id))
            .size(11)
            .font(ui_font(iced::font::Weight::Semibold)),
    )
    .width(Length::Fill)
    .height(Length::Fixed(TITLE_BAR_HEIGHT))
    .center_y(Length::Fill);
    let title_bar: Element<'a, Message> = if controller.chrome_style == DockChromeStyle::Card {
        dock_card_title_bar(title, tokens)
    } else {
        container(title)
            .width(Length::Fill)
            .height(Length::Fixed(TITLE_BAR_HEIGHT))
            .padding([0.0, 6.0])
            .style(move |_theme| {
                iced::widget::container::Style::default()
                    .background(tokens.colors.surface)
                    .border(dock_insert_preview_border(tokens))
            })
            .into()
    };

    if controller.chrome_style == DockChromeStyle::Card {
        dock_card_shell(column![title_bar, body].height(Length::Fill), tokens)
    } else {
        column![title_bar, body]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

fn dock_drag_preview_view<'a, Message>(
    id: &DockId,
    controller: &DockController,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let title = container(
        text(dock_item_title(controller, id))
            .size(11)
            .font(ui_font(iced::font::Weight::Semibold)),
    )
    .width(Length::Fill)
    .height(Length::Fixed(WINDOW_TITLE_BAR_HEIGHT))
    .padding([0.0, 8.0])
    .center_y(Length::Fill)
    .style(move |_theme| {
        iced::widget::container::Style::default()
            .background(tokens.colors.surface)
            .color(tokens.colors.text)
    });
    let body = container(space())
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(tokens.colors.subtle)
                .border(dock_placeholder_border(tokens))
        });
    container(column![title, body])
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(tokens.colors.surface)
                .color(tokens.colors.text)
                .border(dock_placeholder_border(tokens))
        })
        .into()
}

fn dock_placeholder_body<'a, Message>(
    tokens: ThemeTokens,
    chrome_style: DockChromeStyle,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let body = container(space())
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(tokens.colors.accent_soft)
                .color(tokens.colors.text)
                .border(dock_insert_preview_border(tokens))
        });
    if chrome_style == DockChromeStyle::Card {
        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([6.0, 8.0])
            .clip(true)
            .style(move |_theme| {
                iced::widget::container::Style::default()
                    .background(tokens.colors.surface)
                    .color(tokens.colors.text)
            })
            .into()
    } else {
        body.into()
    }
}

fn dock_placeholder_border(tokens: ThemeTokens) -> iced::Border {
    iced::Border {
        color: tokens.colors.border_soft,
        width: 1.0,
        radius: 0.0.into(),
    }
}

fn dock_insert_preview_border(tokens: ThemeTokens) -> iced::Border {
    iced::Border {
        color: tokens.colors.accent_on_soft,
        width: 1.0,
        radius: 0.0.into(),
    }
}

fn dock_drag_handle<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    id: &DockId,
    surface: DockSurfaceId,
    on_action: impl Fn(DockAction) -> Message + Copy + 'a,
    reset: DockAction,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    DragHandle::new(
        content,
        on_action(DockAction::DragStart {
            surface,
            id: id.clone(),
        }),
        move |position| on_action(DockAction::DragMove { surface, position }),
        on_action(DockAction::DragEnd { surface }),
        on_action(reset),
        move |hovered| on_action(DockAction::Hover(hovered)),
        iced::mouse::Interaction::Grabbing,
    )
    .keep_drag_on_unfocused()
    .into()
}

fn dock_item_body<'a, Message>(
    id: &DockId,
    contents: &mut DockContents<'a, Message>,
    tokens: ThemeTokens,
    chrome_style: DockChromeStyle,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let content = contents
        .items
        .remove(id)
        .unwrap_or_else(|| container(space()).into());
    match chrome_style {
        DockChromeStyle::Card => container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([6.0, 8.0])
            .clip(true)
            .style(move |_theme| {
                iced::widget::container::Style::default()
                    .background(tokens.colors.surface)
                    .color(tokens.colors.text)
            })
            .into(),
        DockChromeStyle::Segmented | DockChromeStyle::Borderless => container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .clip(true)
            .style(move |_theme| {
                iced::widget::container::Style::default()
                    .background(tokens.colors.surface)
                    .color(tokens.colors.text)
                    .border(dock_chrome_border(tokens, chrome_style))
            })
            .into(),
    }
}

fn dock_card_shell<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: 'a,
{
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(card_style(tokens, CardKind::Outlined))
        .into()
}

fn dock_card_title_bar<'a, Message>(
    title_bar: impl Into<Element<'a, Message>>,
    tokens: ThemeTokens,
) -> Element<'a, Message>
where
    Message: 'a,
{
    container(title_bar)
        .width(Length::Fill)
        .height(Length::Fixed(TITLE_BAR_HEIGHT))
        .padding([0.0, 8.0])
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(tokens.colors.surface)
                .color(tokens.colors.text)
        })
        .into()
}

fn dock_tab_border(tokens: ThemeTokens, chrome_style: DockChromeStyle) -> iced::Border {
    match chrome_style {
        DockChromeStyle::Card => iced::Border::default().rounded(tokens.metrics.radius_sm),
        DockChromeStyle::Segmented | DockChromeStyle::Borderless => {
            dock_chrome_border(tokens, chrome_style)
        }
    }
}

fn dock_chrome_border(tokens: ThemeTokens, chrome_style: DockChromeStyle) -> iced::Border {
    match chrome_style {
        DockChromeStyle::Segmented => iced::Border {
            color: tokens.colors.border,
            width: 1.0,
            radius: 0.0.into(),
        },
        DockChromeStyle::Borderless | DockChromeStyle::Card => iced::Border::default(),
    }
}

fn dock_chrome_color(tokens: ThemeTokens, chrome_style: DockChromeStyle) -> iced::Color {
    match chrome_style {
        DockChromeStyle::Segmented => tokens.colors.border,
        DockChromeStyle::Borderless | DockChromeStyle::Card => iced::Color::TRANSPARENT,
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

fn remove_view_node(node: DockViewNode, id: &DockId) -> Option<DockViewNode> {
    match node {
        DockViewNode::Item { item } => (item.id() != id).then_some(DockViewNode::Item { item }),
        DockViewNode::Tabs {
            mut tabs,
            mut active,
            ..
        } => {
            let before = tabs.len();
            tabs.retain(|item| item.id() != id);
            if tabs.len() == before {
                return Some(DockViewNode::Tabs { tabs, active });
            }
            match tabs.len() {
                0 => None,
                1 => Some(DockViewNode::Item {
                    item: tabs.remove(0),
                }),
                _ => {
                    if active.id() == id {
                        active = tabs[0].clone();
                    }
                    Some(DockViewNode::Tabs { tabs, active })
                }
            }
        }
        DockViewNode::Split {
            axis,
            ratio,
            first,
            second,
        } => match (remove_view_node(*first, id), remove_view_node(*second, id)) {
            (Some(first), Some(second)) => Some(DockViewNode::Split {
                axis,
                ratio,
                first: Box::new(first),
                second: Box::new(second),
            }),
            (Some(node), None) | (None, Some(node)) => Some(node),
            (None, None) => None,
        },
    }
}

fn insert_view_node(
    root: &mut DockViewNode,
    target: &DockId,
    item: DockViewItem,
    zone: DockDropZone,
) -> bool {
    if root.contains(target)
        && matches!(root, DockViewNode::Item { .. } | DockViewNode::Tabs { .. })
    {
        if zone == DockDropZone::Tab {
            return insert_view_tab(root, target, item);
        }
        let previous = root.clone();
        let item = DockViewNode::Item { item };
        let (axis, first, second) = match zone {
            DockDropZone::Left => (DockAxis::Horizontal, item, previous),
            DockDropZone::Right => (DockAxis::Horizontal, previous, item),
            DockDropZone::Top => (DockAxis::Vertical, item, previous),
            DockDropZone::Bottom => (DockAxis::Vertical, previous, item),
            DockDropZone::Tab => unreachable!("tab insertion handled above"),
        };
        *root = DockViewNode::Split {
            axis,
            ratio: 0.5,
            first: Box::new(first),
            second: Box::new(second),
        };
        return true;
    }
    match root {
        DockViewNode::Split { first, second, .. } => {
            insert_view_node(first, target, item.clone(), zone)
                || insert_view_node(second, target, item, zone)
        }
        _ => false,
    }
}

fn insert_view_tab(root: &mut DockViewNode, target: &DockId, item: DockViewItem) -> bool {
    match root {
        DockViewNode::Item { item: current } if current.id() == target => {
            *root = DockViewNode::Tabs {
                tabs: vec![current.clone(), item.clone()],
                active: item,
            };
            true
        }
        DockViewNode::Tabs { tabs, active } if tabs.iter().any(|tab| tab.id() == target) => {
            if !tabs.iter().any(|tab| tab == &item) {
                tabs.push(item.clone());
            }
            *active = item;
            true
        }
        DockViewNode::Split { first, second, .. } => {
            insert_view_tab(first, target, item.clone()) || insert_view_tab(second, target, item)
        }
        _ => false,
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

fn collect_view_drop_targets(
    node: &DockViewNode,
    bounds: DockBounds,
    output: &mut Vec<(DockId, DockBounds)>,
) {
    match node {
        DockViewNode::Item { item } => output.push((item.id().clone(), bounds)),
        DockViewNode::Tabs { active, .. } => output.push((active.id().clone(), bounds)),
        DockViewNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let (first_bounds, second_bounds) = split_child_bounds(*axis, *ratio, bounds);
            collect_view_drop_targets(first, first_bounds, output);
            collect_view_drop_targets(second, second_bounds, output);
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

    fn preview_controller(zone: DockDropZone) -> DockController {
        let mut controller = simple_drag_controller();
        let target = DockDropTarget {
            surface: DockSurfaceId(0),
            id: DockId::from("editor"),
            zone,
        };
        controller.active_drag = Some(ActiveDrag {
            surface: DockSurfaceId(0),
            id: DockId::from("source"),
            start: None,
            position: None,
            moved: true,
            pending_target: None,
            target: Some(target),
            transient_surface: None,
            transient_ready: false,
            original_bounds: None,
            bounds: None,
        });
        controller
    }

    #[test]
    fn drag_preview_recreates_each_split_zone_without_mutating_layout() {
        let cases = [
            (DockDropZone::Left, DockAxis::Horizontal, true),
            (DockDropZone::Right, DockAxis::Horizontal, false),
            (DockDropZone::Top, DockAxis::Vertical, true),
            (DockDropZone::Bottom, DockAxis::Vertical, false),
        ];
        for (zone, expected_axis, placeholder_first) in cases {
            let controller = preview_controller(zone);
            let before = controller.layout().clone();
            let preview = controller.preview_root().expect("drag preview");
            assert_eq!(controller.layout(), &before);
            let DockViewNode::Split {
                axis,
                ratio,
                first,
                second,
            } = preview
            else {
                panic!("split preview expected")
            };
            assert_eq!(axis, expected_axis);
            assert_eq!(ratio, 0.5);
            let (placeholder, target) = if placeholder_first {
                (&first, &second)
            } else {
                (&second, &first)
            };
            assert_eq!(
                placeholder.as_ref(),
                &DockViewNode::Item {
                    item: DockViewItem::Placeholder(DockId::from("source")),
                }
            );
            assert_eq!(
                target.as_ref(),
                &DockViewNode::Item {
                    item: DockViewItem::Existing(DockId::from("editor")),
                }
            );
        }
    }

    #[test]
    fn drag_preview_tabs_make_the_empty_item_active() {
        let controller = preview_controller(DockDropZone::Tab);
        let preview = controller.preview_root().expect("tab preview");
        let DockViewNode::Tabs { tabs, active } = preview else {
            panic!("tab preview expected")
        };
        assert_eq!(
            tabs,
            vec![
                DockViewItem::Existing(DockId::from("editor")),
                DockViewItem::Placeholder(DockId::from("source")),
            ]
        );
        assert_eq!(active, DockViewItem::Placeholder(DockId::from("source")));
    }

    #[test]
    fn cancelling_drag_removes_preview_without_changing_layout() {
        let mut controller = preview_controller(DockDropZone::Left);
        let before = controller.layout().clone();
        assert!(controller.preview_root().is_some());
        controller.update(DockAction::CancelDrag);
        assert!(controller.preview_root().is_none());
        assert_eq!(controller.layout(), &before);
    }

    #[test]
    fn drag_frame_is_needed_only_during_candidate_dwell() {
        let mut controller = tab_drag_controller();
        let now = iced::time::Instant::now();
        assert!(!controller.is_drag_frame_needed());

        controller.update_at(
            DockAction::DragStart {
                surface: DockSurfaceId(0),
                id: DockId::from("source"),
            },
            now,
        );
        assert!(!controller.is_drag_frame_needed());

        move_source_to_position(&mut controller, now, Point::new(50.0, 400.0));
        assert!(controller.is_drag_frame_needed());

        let preview_ready_at = after_drag_dwell(now);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        assert!(controller.drop_target().is_some());
        assert!(!controller.is_drag_frame_needed());
        assert!(!controller.is_drag_animation_active());
    }

    #[test]
    fn main_drag_keeps_layout_json_unchanged_while_opening_a_transient_surface() {
        let mut controller = preview_controller(DockDropZone::Left);
        let before = controller.layout().clone();
        let json = controller.layout_json().expect("layout json");
        controller.update(DockAction::DragStart {
            surface: DockSurfaceId(0),
            id: DockId::from("source"),
        });
        controller.update(DockAction::DragMove {
            surface: DockSurfaceId(0),
            position: Point::new(0.0, 0.0),
        });
        let update = controller.update(DockAction::DragMove {
            surface: DockSurfaceId(0),
            position: Point::new(100.0, 400.0),
        });
        let DockHostEffect::OpenFloating(floating) = &update.effects[0] else {
            panic!("main drag opens a transient floating surface")
        };
        assert_eq!(controller.layout(), &before);
        assert_eq!(controller.layout_json().expect("layout json"), json);
        assert!(controller.drag_floating(floating.surface).is_some());
        assert!(controller.preview_root().is_some());
    }

    fn simple_drag_controller() -> DockController {
        let layout = DockLayout::new(DockNode::split(
            DockAxis::Horizontal,
            0.5,
            DockNode::item("source"),
            DockNode::item("editor"),
        ));
        DockController::new(
            "editor",
            [
                DockItemSpec::new("editor", "Editor").closeable(false),
                DockItemSpec::new("source", "Source"),
            ],
            layout,
        )
        .expect("valid drag dock layout")
    }

    fn tab_drag_controller() -> DockController {
        let layout = DockLayout::new(DockNode::split(
            DockAxis::Horizontal,
            0.25,
            DockNode::item("source"),
            DockNode::split(
                DockAxis::Horizontal,
                0.5,
                DockNode::item("target"),
                DockNode::item("editor"),
            ),
        ));
        DockController::new(
            "editor",
            [
                DockItemSpec::new("editor", "Editor").closeable(false),
                DockItemSpec::new("source", "Source"),
                DockItemSpec::new("target", "Target"),
            ],
            layout,
        )
        .expect("valid tab drag dock layout")
    }

    fn floating_pair_controller() -> (DockController, DockSurfaceId, DockSurfaceId) {
        let mut controller = controller();
        let source = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(1_400.0, 100.0, 360.0, 280.0),
            monitor: Some("display-2".into()),
        });
        let DockHostEffect::OpenFloating(source) = &source.effects[0] else {
            panic!("source floating window")
        };
        let target = controller.update(DockAction::Float {
            id: "mixer".into(),
            bounds: DockBounds::new(1_900.0, 100.0, 360.0, 280.0),
            monitor: Some("display-2".into()),
        });
        let DockHostEffect::OpenFloating(target) = &target.effects[0] else {
            panic!("target floating window")
        };
        (controller, source.surface, target.surface)
    }

    fn grouped_floating_controller() -> (DockController, DockSurfaceId) {
        let mut controller = controller();
        let opened = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(1_400.0, 100.0, 360.0, 280.0),
            monitor: Some("display-2".into()),
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("floating window")
        };
        assert!(remove_from_layout(&mut controller.layout, &DockId::from("mixer")).0);
        controller.layout.floating[0].root =
            DockNode::tabs([DockId::from("sources"), DockId::from("mixer")], "sources");
        (controller, floating.surface)
    }

    fn grouped_floating_pair_controller() -> (DockController, DockSurfaceId, DockSurfaceId) {
        let mut controller = controller();
        let source = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(1_400.0, 100.0, 360.0, 280.0),
            monitor: Some("display-2".into()),
        });
        let DockHostEffect::OpenFloating(source) = &source.effects[0] else {
            panic!("source floating window")
        };
        let target = controller.update(DockAction::Float {
            id: "mixer".into(),
            bounds: DockBounds::new(1_900.0, 100.0, 360.0, 280.0),
            monitor: Some("display-2".into()),
        });
        let DockHostEffect::OpenFloating(target) = &target.effects[0] else {
            panic!("target floating window")
        };
        assert!(remove_from_layout(&mut controller.layout, &DockId::from("controls")).0);
        controller.layout.floating[0].root = DockNode::tabs(
            [DockId::from("sources"), DockId::from("controls")],
            "sources",
        );
        (controller, source.surface, target.surface)
    }

    fn move_source_to_position(
        controller: &mut DockController,
        now: iced::time::Instant,
        position: Point,
    ) -> DockUpdate {
        controller.update_at(
            DockAction::DragStart {
                surface: DockSurfaceId(0),
                id: DockId::from("source"),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(0.0, 0.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position,
            },
            now + iced::time::Duration::from_millis(1),
        )
    }

    const DRAG_TEST_TICK: iced::time::Duration = iced::time::Duration::from_millis(1);

    fn after_drag_dwell(at: iced::time::Instant) -> iced::time::Instant {
        at + DRAG_INSERT_HOVER_DELAY + DRAG_TEST_TICK
    }

    #[test]
    fn real_tab_drop_commits_the_preview_layout() {
        let mut controller = tab_drag_controller();
        let now = iced::time::Instant::now();
        let opened = move_source_to_position(&mut controller, now, Point::new(300.0, 400.0));
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("tab drag opens a transient floating surface")
        };
        let update = controller.update_at(
            DockAction::DragEnd {
                surface: DockSurfaceId(0),
            },
            after_drag_dwell(now),
        );
        assert!(update.changed);
        assert_eq!(
            update.effects,
            vec![DockHostEffect::CloseFloating(floating.surface)]
        );

        let DockNode::Split { first, .. } = &controller.layout().main else {
            panic!("source removal should preserve the target split")
        };
        let DockNode::Tabs { tabs, active } = first.as_ref() else {
            panic!("tab drop should commit a tabs node")
        };
        assert_eq!(tabs, &vec![DockId::from("target"), DockId::from("source")]);
        assert_eq!(active, &DockId::from("source"));
        assert!(controller.layout().floating.is_empty());
    }

    #[test]
    fn changing_candidate_clears_old_preview_until_new_dwell_finishes() {
        let mut controller = tab_drag_controller();
        let now = iced::time::Instant::now();
        move_source_to_position(&mut controller, now, Point::new(50.0, 400.0));
        controller.update_at(DockAction::Hover(false), after_drag_dwell(now));
        assert_eq!(
            controller.drop_target().map(|target| target.zone),
            Some(DockDropZone::Left)
        );

        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(300.0, 400.0),
            },
            after_drag_dwell(now) + DRAG_TEST_TICK,
        );
        assert!(controller.drop_target().is_none());
        let retargeted_at = after_drag_dwell(now) + DRAG_TEST_TICK;
        let cleared = controller.preview_root().expect("drag preview");
        assert!(!contains_placeholder(&cleared));
        assert!(!controller.is_drag_animation_active());

        controller.update_at(DockAction::Hover(false), after_drag_dwell(retargeted_at));
        assert_eq!(
            controller.drop_target().map(|target| target.zone),
            Some(DockDropZone::Tab)
        );
        let tab_preview = controller.preview_root().expect("tab preview");
        let DockViewNode::Split { first, .. } = tab_preview else {
            panic!("target split should remain in the preview root")
        };
        let DockViewNode::Tabs { tabs, active } = first.as_ref() else {
            panic!("tab target should have a direct tab preview")
        };
        assert_eq!(
            tabs.last(),
            Some(&DockViewItem::Placeholder(DockId::from("source")))
        );
        assert_eq!(active, &DockViewItem::Placeholder(DockId::from("source")));
    }

    #[test]
    fn rapid_candidate_changes_commit_only_the_latest_preview_target() {
        let mut controller = tab_drag_controller();
        let before = controller.layout().clone();
        let before_json = controller.layout_json().expect("layout json");
        let now = iced::time::Instant::now();

        let opened = move_source_to_position(&mut controller, now, Point::new(50.0, 400.0));
        assert!(matches!(
            opened.effects.first(),
            Some(DockHostEffect::OpenFloating(_))
        ));
        controller.update_at(DockAction::Hover(false), after_drag_dwell(now));
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: DockSurfaceId(0),
                id: DockId::from("target"),
                zone: DockDropZone::Left,
            })
        );

        let retargeted_at = after_drag_dwell(now) + DRAG_TEST_TICK;
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(300.0, 400.0),
            },
            retargeted_at,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(630.0, 400.0),
            },
            retargeted_at + DRAG_TEST_TICK,
        );
        assert!(controller.drop_target().is_none());
        assert_eq!(
            controller.drop_highlight_target(),
            Some(&DockDropTarget {
                surface: DockSurfaceId(0),
                id: DockId::from("target"),
                zone: DockDropZone::Right,
            })
        );

        let preview_ready_at = after_drag_dwell(retargeted_at + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: DockSurfaceId(0),
                id: DockId::from("target"),
                zone: DockDropZone::Right,
            })
        );
        assert_eq!(controller.layout(), &before);
        assert_eq!(controller.layout_json().expect("layout json"), before_json);

        let preview = controller.preview_root().expect("latest target preview");
        let DockViewNode::Split { first, .. } = preview else {
            panic!("latest target preview should preserve the target split")
        };
        let DockViewNode::Split { ratio, second, .. } = first.as_ref() else {
            panic!("latest target preview should be a nested split")
        };
        assert_eq!(*ratio, 0.5);
        assert_eq!(
            second.as_ref(),
            &DockViewNode::Item {
                item: DockViewItem::Placeholder(DockId::from("source")),
            }
        );

        let update = controller.update_at(
            DockAction::DragEnd {
                surface: DockSurfaceId(0),
            },
            preview_ready_at + DRAG_TEST_TICK,
        );
        assert!(update.changed);
        assert!(controller.layout().main.contains(&DockId::from("source")));
    }

    #[test]
    fn cross_surface_preview_does_not_restore_the_old_surface_target() {
        let (mut controller, source, target) = floating_pair_controller();
        let before_json = controller.layout_json().expect("layout json");
        let now = iced::time::Instant::now();
        controller.update_at(
            DockAction::DragStart {
                surface: source,
                id: DockId::from("sources"),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: source,
                position: Point::new(100.0, 100.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: target,
                position: Point::new(180.0, 140.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        controller.update_at(DockAction::Hover(false), after_drag_dwell(now));
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: target,
                id: DockId::from("mixer"),
                zone: DockDropZone::Tab,
            })
        );

        let retargeted_at = after_drag_dwell(now) + DRAG_TEST_TICK;
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(300.0, 400.0),
            },
            retargeted_at,
        );
        let latest_target_at = retargeted_at + DRAG_TEST_TICK;
        controller.update_at(
            DockAction::DragMove {
                surface: target,
                position: Point::new(180.0, 140.0),
            },
            latest_target_at,
        );
        let preview_ready_at = after_drag_dwell(latest_target_at);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: target,
                id: DockId::from("mixer"),
                zone: DockDropZone::Tab,
            })
        );
        assert_eq!(controller.layout_json().expect("layout json"), before_json);

        assert!(controller.preview_root_for(source).is_none());
        let target_surface = controller
            .preview_root_for(target)
            .expect("target surface preview");
        assert!(contains_placeholder(&target_surface));
    }

    #[test]
    fn leaving_and_reentering_before_dwell_only_settles_the_reentered_target() {
        let mut controller = simple_drag_controller();
        let before = controller.layout().clone();
        let before_json = controller.layout_json().expect("layout json");
        let now = iced::time::Instant::now();
        move_source_to_position(&mut controller, now, Point::new(100.0, 400.0));
        assert!(controller.drop_target().is_none());

        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(700.0, 400.0),
            },
            now + iced::time::Duration::from_millis(100),
        );
        assert!(controller.drop_highlight_target().is_none());
        assert!(!contains_placeholder(
            &controller.preview_root().expect("drag preview")
        ));

        let reentered_at = now + iced::time::Duration::from_millis(151);
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(100.0, 400.0),
            },
            reentered_at,
        );
        let reentered_ready_at = reentered_at + DRAG_INSERT_HOVER_DELAY;
        controller.update_at(DockAction::Hover(false), reentered_ready_at);
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: DockSurfaceId(0),
                id: DockId::from("editor"),
                zone: DockDropZone::Left,
            })
        );
        assert_eq!(controller.layout(), &before);
        assert_eq!(controller.layout_json().expect("layout json"), before_json);
    }

    #[test]
    fn dock_insert_target_requires_an_80ms_dwell_after_drag_threshold() {
        let mut controller = simple_drag_controller();
        let now = iced::time::Instant::now();
        controller.update_at(
            DockAction::DragStart {
                surface: DockSurfaceId(0),
                id: DockId::from("source"),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(0.0, 0.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(2.0, 2.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        assert!(controller.drop_target().is_none());
        assert!(controller.drop_highlight_target().is_none());

        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(100.0, 400.0),
            },
            now + iced::time::Duration::from_millis(2),
        );
        assert!(controller.drop_target().is_none());
        assert_eq!(
            controller.drop_highlight_target(),
            Some(&DockDropTarget {
                surface: DockSurfaceId(0),
                id: DockId::from("editor"),
                zone: DockDropZone::Left,
            })
        );

        controller.update_at(
            DockAction::Hover(false),
            now + DRAG_INSERT_HOVER_DELAY + iced::time::Duration::from_millis(1),
        );
        assert!(controller.drop_target().is_none());
        controller.update_at(
            DockAction::Hover(false),
            now + DRAG_INSERT_HOVER_DELAY + iced::time::Duration::from_millis(2),
        );
        assert!(controller.drop_target().is_some());
    }

    #[test]
    fn changing_or_leaving_a_candidate_resets_or_cancels_the_dwell() {
        let mut controller = simple_drag_controller();
        let before = controller.layout().clone();
        let now = iced::time::Instant::now();
        move_source_to_position(&mut controller, now, Point::new(100.0, 400.0));
        assert!(controller.drop_target().is_none());

        let changed_at = now + DRAG_INSERT_HOVER_DELAY;
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(1200.0, 400.0),
            },
            changed_at,
        );
        assert_eq!(
            controller.drop_highlight_target().map(|target| target.zone),
            Some(DockDropZone::Right)
        );
        controller.update_at(
            DockAction::Hover(false),
            changed_at + DRAG_INSERT_HOVER_DELAY,
        );
        assert!(controller.drop_target().is_some());

        let left_at = now + iced::time::Duration::from_millis(600);
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(700.0, 400.0),
            },
            left_at,
        );
        assert!(controller.drop_highlight_target().is_none());
        assert!(controller.drop_target().is_none());
        assert!(!contains_placeholder(
            &controller.preview_root().expect("drag preview")
        ));
        assert!(!controller.is_drag_animation_active());
        assert_eq!(controller.layout(), &before);
    }

    #[test]
    fn releasing_before_dwell_keeps_the_drag_floating_but_deadline_release_docks() {
        let now = iced::time::Instant::now();
        let mut before_deadline = simple_drag_controller();
        let opened = move_source_to_position(&mut before_deadline, now, Point::new(100.0, 400.0));
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("transient surface")
        };
        let floating_surface = floating.surface;
        before_deadline.update_at(
            DockAction::DragEnd {
                surface: DockSurfaceId(0),
            },
            now + DRAG_INSERT_HOVER_DELAY,
        );
        assert_eq!(before_deadline.layout().floating.len(), 1);
        assert_eq!(
            before_deadline.layout().floating[0].surface,
            floating_surface
        );

        let mut at_deadline = simple_drag_controller();
        let opened = move_source_to_position(&mut at_deadline, now, Point::new(100.0, 400.0));
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("transient surface")
        };
        let update = at_deadline.update_at(
            DockAction::DragEnd {
                surface: DockSurfaceId(0),
            },
            after_drag_dwell(now),
        );
        assert!(update.changed);
        assert_eq!(
            update.effects,
            vec![DockHostEffect::CloseFloating(floating.surface)]
        );
        assert!(at_deadline.layout().floating.is_empty());
        assert!(at_deadline.layout().main.contains(&DockId::from("source")));
    }

    #[test]
    fn dropping_into_main_closes_the_transient_surface_without_recreating_it() {
        let mut controller = preview_controller(DockDropZone::Left);
        let now = iced::time::Instant::now();
        controller.update_at(
            DockAction::DragStart {
                surface: DockSurfaceId(0),
                id: DockId::from("source"),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(0.0, 0.0),
            },
            now,
        );
        let opened = controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(100.0, 400.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        assert!(controller.drop_target().is_none());
        let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: DockSurfaceId(0),
                id: DockId::from("editor"),
                zone: DockDropZone::Left,
            })
        );
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("transient surface")
        };
        let update = controller.update_at(
            DockAction::DragEnd {
                surface: DockSurfaceId(0),
            },
            preview_ready_at + iced::time::Duration::from_millis(1),
        );
        assert!(update.changed);
        assert_eq!(
            update.effects,
            vec![DockHostEffect::CloseFloating(floating.surface)]
        );
        assert!(controller.layout().floating.is_empty());
        assert!(controller.layout().main.contains(&DockId::from("source")));
        assert!(!controller.is_dragging());
    }

    #[test]
    fn releasing_outside_promotes_the_same_transient_surface_to_persistent_floating() {
        let mut controller = preview_controller(DockDropZone::Left);
        controller.update(DockAction::DragStart {
            surface: DockSurfaceId(0),
            id: DockId::from("source"),
        });
        controller.update(DockAction::DragMove {
            surface: DockSurfaceId(0),
            position: Point::new(0.0, 0.0),
        });
        let opened = controller.update(DockAction::DragMove {
            surface: DockSurfaceId(0),
            position: Point::new(700.0, 400.0),
        });
        assert_eq!(controller.drop_target(), None);
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("transient surface")
        };
        let update = controller.update(DockAction::DragEnd {
            surface: DockSurfaceId(0),
        });
        assert!(update.changed);
        assert!(update.effects.is_empty());
        assert_eq!(controller.layout().floating.len(), 1);
        assert_eq!(controller.layout().floating[0].surface, floating.surface);
        assert_eq!(
            controller.layout().floating[0].root,
            DockNode::item("source")
        );
    }

    #[test]
    fn cancelling_a_floating_drag_restores_only_the_host_position() {
        let mut controller = controller();
        let opened = controller.update(DockAction::Float {
            id: DockId::from("sources"),
            bounds: DockBounds::new(40.0, 50.0, 360.0, 280.0),
            monitor: None,
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("floating surface")
        };
        let before = controller.layout().clone();
        controller.update(DockAction::DragStart {
            surface: floating.surface,
            id: DockId::from("sources"),
        });
        controller.update(DockAction::DragMove {
            surface: floating.surface,
            position: Point::new(100.0, 100.0),
        });
        controller.update(DockAction::DragMove {
            surface: floating.surface,
            position: Point::new(-600.0, -600.0),
        });
        let update = controller.update(DockAction::CancelDrag);
        assert_eq!(controller.layout(), &before);
        assert_eq!(
            update.effects,
            vec![DockHostEffect::MoveFloating {
                surface: floating.surface,
                bounds: floating.bounds,
            }]
        );
        assert!(!controller.is_dragging());
    }

    #[test]
    fn floating_drag_can_dock_into_main_and_close_the_source_surface() {
        let mut controller = controller();
        let opened = controller.update(DockAction::Float {
            id: DockId::from("sources"),
            bounds: DockBounds::new(1_400.0, 100.0, 360.0, 280.0),
            monitor: None,
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("floating surface")
        };
        let surface = floating.surface;
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(100.0, 50.0, 1_280.0, 800.0),
        });

        let now = iced::time::Instant::now();
        controller.update_at(
            DockAction::DragStart {
                surface,
                id: DockId::from("sources"),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface,
                position: Point::new(10.0, 10.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(150.0, 250.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        let update = controller.update_at(
            DockAction::DragEnd {
                surface: DockSurfaceId(0),
            },
            preview_ready_at + iced::time::Duration::from_millis(1),
        );
        assert!(update.changed);
        assert_eq!(update.effects, vec![DockHostEffect::CloseFloating(surface)]);
        assert!(controller.layout().floating.is_empty());
        assert!(!controller.is_dragging());
        let DockNode::Split { first, .. } = &controller.layout().main else {
            panic!("main layout")
        };
        assert_eq!(
            first.as_ref(),
            &DockNode::Tabs {
                tabs: vec![DockId::from("scenes"), DockId::from("sources")],
                active: DockId::from("sources"),
            }
        );
    }

    #[test]
    fn floating_drag_can_merge_into_another_floating_surface() {
        let (mut controller, source, target) = floating_pair_controller();
        let now = iced::time::Instant::now();
        controller.update_at(
            DockAction::DragStart {
                surface: source,
                id: "sources".into(),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: source,
                position: Point::new(100.0, 100.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: target,
                position: Point::new(180.0, 140.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: target,
                id: DockId::from("mixer"),
                zone: DockDropZone::Tab,
            })
        );
        let update = controller.update_at(
            DockAction::DragEnd { surface: target },
            preview_ready_at + iced::time::Duration::from_millis(1),
        );

        assert!(update.changed);
        assert_eq!(update.effects, vec![DockHostEffect::CloseFloating(source)]);
        assert_eq!(controller.layout().floating.len(), 1);
        assert_eq!(controller.layout().floating[0].surface, target);
        assert_eq!(
            controller.layout().floating[0].root,
            DockNode::tabs([DockId::from("mixer"), DockId::from("sources")], "sources")
        );
        assert!(!controller.is_dragging());
    }

    #[test]
    fn floating_drag_can_split_inside_another_floating_surface_on_each_edge() {
        let cases = [
            (DockDropZone::Left, Point::new(20.0, 140.0), true),
            (DockDropZone::Right, Point::new(340.0, 140.0), false),
            (DockDropZone::Top, Point::new(180.0, 20.0), true),
            (DockDropZone::Bottom, Point::new(180.0, 260.0), false),
        ];
        for (zone, target_position, inserted_first) in cases {
            let (mut controller, source, target) = floating_pair_controller();
            let now = iced::time::Instant::now();
            controller.update_at(
                DockAction::DragStart {
                    surface: source,
                    id: "sources".into(),
                },
                now,
            );
            controller.update_at(
                DockAction::DragMove {
                    surface: source,
                    position: Point::new(100.0, 100.0),
                },
                now,
            );
            controller.update_at(
                DockAction::DragMove {
                    surface: target,
                    position: target_position,
                },
                now + iced::time::Duration::from_millis(1),
            );
            let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
            controller.update_at(DockAction::Hover(false), preview_ready_at);
            assert_eq!(
                controller.drop_target().map(|target| target.zone),
                Some(zone)
            );
            let update = controller.update_at(
                DockAction::DragEnd { surface: target },
                preview_ready_at + iced::time::Duration::from_millis(1),
            );
            assert_eq!(update.effects, vec![DockHostEffect::CloseFloating(source)]);

            let DockNode::Split {
                axis,
                first,
                second,
                ..
            } = &controller.layout().floating[0].root
            else {
                panic!("floating edge drop should create a split")
            };
            assert_eq!(
                *axis,
                match zone {
                    DockDropZone::Left | DockDropZone::Right => DockAxis::Horizontal,
                    DockDropZone::Top | DockDropZone::Bottom => DockAxis::Vertical,
                    DockDropZone::Tab => unreachable!(),
                }
            );
            let (inserted, existing) = if inserted_first {
                (first.as_ref(), second.as_ref())
            } else {
                (second.as_ref(), first.as_ref())
            };
            assert_eq!(inserted, &DockNode::item("sources"));
            assert_eq!(existing, &DockNode::item("mixer"));
        }
    }

    #[test]
    fn dragging_one_panel_out_of_a_grouped_floating_surface_keeps_the_source_window() {
        let (mut controller, source, target) = grouped_floating_pair_controller();
        let now = iced::time::Instant::now();
        controller.update_at(
            DockAction::DragStart {
                surface: source,
                id: "sources".into(),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: source,
                position: Point::new(100.0, 100.0),
            },
            now,
        );
        let opened = controller.update_at(
            DockAction::DragMove {
                surface: target,
                position: Point::new(180.0, 140.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        let DockHostEffect::OpenFloating(transient) = &opened.effects[0] else {
            panic!("grouped floating drag opens a transient panel surface")
        };
        assert_ne!(transient.surface, source);
        assert_eq!(controller.layout().floating.len(), 2);
        assert_eq!(
            controller.layout().floating[0].root,
            DockNode::tabs(
                [DockId::from("sources"), DockId::from("controls")],
                "sources"
            )
        );

        let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        let update = controller.update_at(
            DockAction::DragEnd { surface: target },
            preview_ready_at + iced::time::Duration::from_millis(1),
        );
        assert_eq!(
            update.effects,
            vec![DockHostEffect::CloseFloating(transient.surface)]
        );
        assert_eq!(controller.layout().floating.len(), 2);
        assert_eq!(controller.layout().floating[0].surface, source);
        assert_eq!(controller.layout().floating[1].surface, target);
        assert_eq!(
            controller.layout().floating[0].root,
            DockNode::item("controls")
        );
        assert_eq!(
            controller.layout().floating[1].root,
            DockNode::tabs([DockId::from("mixer"), DockId::from("sources")], "sources")
        );
    }

    #[test]
    fn cancelling_or_releasing_a_grouped_floating_drag_preserves_panel_ownership() {
        let (mut cancelled, source) = grouped_floating_controller();
        let before = cancelled.layout().clone();
        let now = iced::time::Instant::now();
        cancelled.update_at(
            DockAction::DragStart {
                surface: source,
                id: "sources".into(),
            },
            now,
        );
        cancelled.update_at(
            DockAction::DragMove {
                surface: source,
                position: Point::new(100.0, 100.0),
            },
            now,
        );
        let opened = cancelled.update_at(
            DockAction::DragMove {
                surface: source,
                position: Point::new(108.0, 108.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        let DockHostEffect::OpenFloating(transient) = &opened.effects[0] else {
            panic!("grouped drag opens a transient panel surface")
        };
        let update = cancelled.update(DockAction::CancelDrag);
        assert_eq!(cancelled.layout(), &before);
        assert_eq!(
            update.effects,
            vec![DockHostEffect::CloseFloating(transient.surface)]
        );

        let (mut released, source) = grouped_floating_controller();
        let now = iced::time::Instant::now();
        released.update_at(
            DockAction::DragStart {
                surface: source,
                id: "sources".into(),
            },
            now,
        );
        released.update_at(
            DockAction::DragMove {
                surface: source,
                position: Point::new(100.0, 100.0),
            },
            now,
        );
        let opened = released.update_at(
            DockAction::DragMove {
                surface: source,
                position: Point::new(700.0, 500.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        let DockHostEffect::OpenFloating(transient) = &opened.effects[0] else {
            panic!("grouped drag opens a transient panel surface")
        };
        let update = released.update_at(
            DockAction::DragEnd { surface: source },
            now + iced::time::Duration::from_millis(2),
        );
        assert!(update.changed);
        assert!(update.effects.is_empty());
        assert_eq!(released.layout().floating.len(), 2);
        assert_eq!(released.layout().floating[0].surface, source);
        assert_eq!(released.layout().floating[0].root, DockNode::item("mixer"));
        assert_eq!(released.layout().floating[1].surface, transient.surface);
        assert_eq!(
            released.layout().floating[1].root,
            DockNode::item("sources")
        );
    }

    #[test]
    fn main_surface_pointer_routes_back_to_a_floating_drag_source() {
        let mut controller = controller();
        let opened = controller.update(DockAction::Float {
            id: DockId::from("sources"),
            bounds: DockBounds::new(1_400.0, 100.0, 360.0, 280.0),
            monitor: None,
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("floating surface")
        };
        let surface = floating.surface;
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(100.0, 50.0, 1_280.0, 800.0),
        });
        controller.update(DockAction::DragStart {
            surface,
            id: DockId::from("sources"),
        });

        let pointer = dock_surface_pointer_handler(&controller, DockSurfaceId(0), |action| action)
            .expect("main surface pointer handler");
        assert_eq!(
            pointer(DockSurfacePointer::Move(Point::new(150.0, 200.0))),
            Some(DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(150.0, 200.0),
            })
        );
        assert_eq!(
            pointer(DockSurfacePointer::End),
            Some(DockAction::DragEnd {
                surface: DockSurfaceId(0),
            })
        );
    }

    #[test]
    fn drag_preview_slot_uses_the_final_ratio_without_animation() {
        let mut controller = preview_controller(DockDropZone::Left);
        let preview_ratio = |root: DockViewNode| match root {
            DockViewNode::Split { ratio, .. } => ratio,
            _ => panic!("split preview"),
        };
        assert_eq!(
            preview_ratio(controller.preview_root().expect("preview")),
            0.5
        );
        assert!(!controller.is_drag_animation_active());

        let drag = controller.active_drag.as_mut().expect("drag");
        drag.target = None;
        let gone = controller.preview_root().expect("preview without target");
        assert!(!contains_placeholder(&gone));
    }

    #[test]
    fn cross_surface_pointer_routing_reports_the_receiving_surface() {
        let mut controller = preview_controller(DockDropZone::Left);
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(100.0, 50.0, 1_280.0, 800.0),
        });
        controller.update(DockAction::DragStart {
            surface: DockSurfaceId(0),
            id: DockId::from("source"),
        });
        controller.update(DockAction::DragMove {
            surface: DockSurfaceId(0),
            position: Point::new(0.0, 0.0),
        });
        let opened = controller.update(DockAction::DragMove {
            surface: DockSurfaceId(0),
            position: Point::new(700.0, 400.0),
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("transient surface")
        };
        let pointer = dock_surface_pointer_handler(&controller, floating.surface, |action| action)
            .expect("cross-surface pointer handler");
        assert_eq!(
            pointer(DockSurfacePointer::Move(Point::new(4.0, 6.0))),
            Some(DockAction::DragMove {
                surface: floating.surface,
                position: Point::new(4.0, 6.0),
            })
        );
        assert_eq!(
            pointer(DockSurfacePointer::End),
            Some(DockAction::DragEnd {
                surface: floating.surface,
            })
        );
    }

    fn contains_placeholder(node: &DockViewNode) -> bool {
        match node {
            DockViewNode::Item { item } => item.is_placeholder(),
            DockViewNode::Tabs { tabs, active, .. } => {
                tabs.iter().any(DockViewItem::is_placeholder) || active.is_placeholder()
            }
            DockViewNode::Split { first, second, .. } => {
                contains_placeholder(first) || contains_placeholder(second)
            }
        }
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
    fn floating_surfaces_accept_tabs_and_splits() {
        let mut controller = controller();
        let opened = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(1_400.0, 40.0, 360.0, 280.0),
            monitor: None,
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("floating window open effect")
        };
        let update = controller.update(DockAction::Dock {
            id: "controls".into(),
            target: DockDropTarget {
                surface: floating.surface,
                id: "sources".into(),
                zone: DockDropZone::Tab,
            },
        });
        assert!(update.changed);
        assert!(update.effects.is_empty());
        assert_eq!(
            controller.layout().floating[0].root,
            DockNode::tabs(
                [DockId::from("sources"), DockId::from("controls")],
                "controls"
            )
        );

        let update = controller.update(DockAction::Dock {
            id: "mixer".into(),
            target: DockDropTarget {
                surface: floating.surface,
                id: "controls".into(),
                zone: DockDropZone::Right,
            },
        });
        assert!(update.changed);
        let DockNode::Split {
            axis: DockAxis::Horizontal,
            first,
            second,
            ..
        } = &controller.layout().floating[0].root
        else {
            panic!("floating edge drop should create a split")
        };
        assert_eq!(
            first.as_ref(),
            &DockNode::tabs(
                [DockId::from("sources"), DockId::from("controls")],
                "controls",
            )
        );
        assert_eq!(second.as_ref(), &DockNode::item("mixer"));
    }

    #[test]
    fn single_item_floating_layout_round_trips() {
        let mut state = controller();
        state.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(40.0, 50.0, 360.0, 280.0),
            monitor: Some("display-2".into()),
        });
        let encoded = state.layout_json().expect("dock layout serializes");
        let mut restored = controller();
        restored
            .restore_layout_json(&encoded)
            .expect("single floating Dock restores");
        assert!(matches!(
            restored.layout().floating.as_slice(),
            [FloatingDock {
                root: DockNode::Item { id },
                ..
            }] if *id == DockId::from("sources")
        ));
    }

    #[test]
    fn grouped_floating_items_round_trip() {
        let mut state = controller();
        let opened = state.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(40.0, 50.0, 360.0, 280.0),
            monitor: None,
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("floating window open effect")
        };
        assert!(remove_from_layout(&mut state.layout, &DockId::from("mixer")).0);
        assert!(remove_from_layout(&mut state.layout, &DockId::from("controls")).0);
        state.layout.floating[0].root = DockNode::split(
            DockAxis::Horizontal,
            0.5,
            DockNode::tabs([DockId::from("sources"), DockId::from("mixer")], "sources"),
            DockNode::item("controls"),
        );
        let encoded = state.layout_json().expect("dock layout serializes");
        let mut restored = controller();
        restored
            .restore_layout_json(&encoded)
            .expect("grouped floating Dock restores");
        assert_eq!(restored.layout().floating[0].surface, floating.surface);
        assert_eq!(
            restored.layout().floating[0].root,
            state.layout.floating[0].root
        );
    }

    #[test]
    fn closing_floating_surface_hides_only_its_item() {
        let mut controller = controller();
        let opened = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(40.0, 50.0, 360.0, 280.0),
            monitor: None,
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("floating window open effect")
        };
        let update = controller.update(DockAction::CloseSurface(floating.surface));
        assert_eq!(
            update.effects,
            vec![DockHostEffect::CloseFloating(floating.surface)]
        );
        assert!(!controller.is_visible(&DockId::from("sources")));
        assert!(controller.is_visible(&DockId::from("scenes")));
        assert!(controller.is_visible(&DockId::from("mixer")));
        assert!(controller.is_visible(&DockId::from("controls")));
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
        let now = iced::time::Instant::now();
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(0.0, 0.0, 1_000.0, 760.0),
        });
        controller.update(DockAction::SurfaceLayout {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(0.0, 60.0, 1_000.0, 700.0),
        });
        controller.update_at(
            DockAction::DragStart {
                surface: floating.surface,
                id: "sources".into(),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: floating.surface,
                position: Point::new(100.0, 80.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(300.0, 120.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: DockSurfaceId(0),
                id: "editor".into(),
                zone: DockDropZone::Left,
            })
        );
        let update = controller.update_at(
            DockAction::DragEnd {
                surface: DockSurfaceId(0),
            },
            preview_ready_at + iced::time::Duration::from_millis(1),
        );
        assert_eq!(
            update.effects,
            vec![DockHostEffect::CloseFloating(floating.surface)]
        );
        assert!(controller.layout().floating.is_empty());
        assert!(controller.is_visible(&DockId::from("sources")));
    }

    #[test]
    fn primary_drop_target_uses_dock_surface_layout_bounds() {
        let mut controller = controller();
        let opened = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(1_400.0, 0.0, 360.0, 280.0),
            monitor: None,
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("floating surface")
        };
        let now = iced::time::Instant::now();
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(0.0, 0.0, 1_000.0, 760.0),
        });
        controller.update(DockAction::SurfaceLayout {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(0.0, 80.0, 1_000.0, 680.0),
        });
        controller.update_at(
            DockAction::DragStart {
                surface: floating.surface,
                id: "sources".into(),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: floating.surface,
                position: Point::new(100.0, 80.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(300.0, 40.0),
            },
            now + iced::time::Duration::from_millis(1),
        );
        assert_eq!(controller.drop_target(), None);

        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: Point::new(300.0, 180.0),
            },
            now + iced::time::Duration::from_millis(2),
        );
        let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: DockSurfaceId(0),
                id: "editor".into(),
                zone: DockDropZone::Left,
            })
        );
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
        let hosted = hosted_dock_update(dock_update.clone(), "NanaUI Dock");
        let HostedWindowCommand::Open { id, settings } = &hosted.window_commands[0] else {
            panic!("hosted open command")
        };
        assert_eq!(*id, HostedWindowId::from(surface));
        assert_eq!(settings.title_bar_mode, crate::HostedTitleBarMode::Native);
        assert_eq!(settings.initial_position, Some((40.0, 50.0)));
        assert_eq!(settings.initial_size, Size::new(360.0, 280.0));
        let moved = hosted_dock_update(
            DockUpdate {
                changed: false,
                effects: vec![DockHostEffect::MoveFloating {
                    surface,
                    bounds: DockBounds::new(80.0, 90.0, 360.0, 280.0),
                }],
            },
            "NanaUI Dock",
        );
        assert!(matches!(
            moved.window_commands.as_slice(),
            [HostedWindowCommand::Move { id, position }]
                if *id == HostedWindowId::from(surface) && *position == Point::new(80.0, 90.0)
        ));
        let restored = controller.open_hosted_windows("NanaUI Dock");
        assert_eq!(restored.window_commands.len(), 1);
        let HostedWindowCommand::Open { settings, .. } = &restored.window_commands[0] else {
            panic!("restored hosted open command")
        };
        assert_eq!(settings.title_bar_mode, crate::HostedTitleBarMode::Native);
        let custom = hosted_dock_update_with_title_bar(
            dock_update,
            "NanaUI Dock",
            crate::HostedTitleBarMode::Custom,
        );
        let HostedWindowCommand::Open { id, settings } = &custom.window_commands[0] else {
            panic!("custom hosted open command")
        };
        assert_eq!(*id, HostedWindowId::from(surface));
        assert_eq!(settings.title_bar_mode, crate::HostedTitleBarMode::Custom);
        let restored_custom = controller
            .open_hosted_windows_with_title_bar("NanaUI Dock", crate::HostedTitleBarMode::Custom);
        let HostedWindowCommand::Open { settings, .. } = &restored_custom.window_commands[0] else {
            panic!("custom restored hosted open command")
        };
        assert_eq!(settings.title_bar_mode, crate::HostedTitleBarMode::Custom);

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
