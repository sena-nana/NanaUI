//! Serde dock model: identities, trees, persisted layout, mutations, errors.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use nana_ui_core::LogicalPoint;
#[cfg(feature = "hosted")]
use nana_ui_platform::WindowId;

pub(super) const DOCK_LAYOUT_VERSION: u8 = 1;
pub(super) const DIVIDER_HIT_SIZE: f32 = 8.0;
pub(super) const TITLE_BAR_HEIGHT: f32 = 28.0;
pub(super) const MIN_SPLIT_RATIO: f32 = 0.05;
pub(super) const MAX_SPLIT_RATIO: f32 = 0.95;
pub(super) const DRAG_INSERT_HOVER_DELAY: Duration = Duration::from_millis(80);
pub(super) const DRAG_CARD_WIDTH: f32 = 280.0;
pub(super) const DRAG_CARD_HEIGHT: f32 = 180.0;
pub(super) const DRAG_CARD_OFFSET: f32 = 12.0;

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
impl From<DockSurfaceId> for WindowId {
    fn from(value: DockSurfaceId) -> Self {
        Self(value.0)
    }
}

#[cfg(feature = "hosted")]
impl From<WindowId> for DockSurfaceId {
    fn from(value: WindowId) -> Self {
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

    pub(super) fn ids(&self, output: &mut Vec<DockId>) {
        match self {
            Self::Item { id } => output.push(id.clone()),
            Self::Tabs { tabs, .. } => output.extend(tabs.iter().cloned()),
            Self::Split { first, second, .. } => {
                first.ids(output);
                second.ids(output);
            }
        }
    }

    pub(super) fn contains(&self, needle: &DockId) -> bool {
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

    pub(super) fn intersection_area(self, other: Self) -> f32 {
        let width = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let height = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        if width <= 0.0 || height <= 0.0 {
            0.0
        } else {
            width * height
        }
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
    ResizeMove(LogicalPoint),
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
        /// Named monitor, or `None` to infer from live work areas.
        monitor: Option<String>,
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
        position: LogicalPoint,
    },
    /// Ends the active drag from the surface that currently owns the pointer.
    DragEnd {
        surface: DockSurfaceId,
    },
    CancelDrag,
    AdvanceDragDwell,
    Hover(bool),
    CardHover(DockId, bool),
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

/// Backend-neutral Dock state mutation. Host adapters convert [`DockAction`] into this contract.
#[derive(Debug, Clone, PartialEq)]
pub enum DockMutation {
    ActivateTab(DockId),
    ReorderTab {
        id: DockId,
        before: Option<DockId>,
    },
    ResizeStart {
        surface: DockSurfaceId,
        path: Vec<usize>,
    },
    ResizeMove(LogicalPoint),
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
        monitor: Option<String>,
    },
    SurfaceLayout {
        surface: DockSurfaceId,
        bounds: DockBounds,
    },
    DragStart {
        surface: DockSurfaceId,
        id: DockId,
    },
    DragMove {
        surface: DockSurfaceId,
        position: LogicalPoint,
    },
    DragEnd {
        surface: DockSurfaceId,
    },
    CancelDrag,
    AdvanceDragDwell,
    CardHover(DockId, bool),
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

impl From<DockAction> for DockMutation {
    fn from(action: DockAction) -> Self {
        match action {
            DockAction::ActivateTab(id) => Self::ActivateTab(id),
            DockAction::ReorderTab { id, before } => Self::ReorderTab { id, before },
            DockAction::ResizeStart { surface, path } => Self::ResizeStart { surface, path },
            DockAction::ResizeMove(position) => {
                Self::ResizeMove(LogicalPoint::new(position.x, position.y))
            }
            DockAction::ResizeEnd => Self::ResizeEnd,
            DockAction::ResizeSplit {
                surface,
                path,
                ratio,
            } => Self::ResizeSplit {
                surface,
                path,
                ratio,
            },
            DockAction::AdjustSplit {
                surface,
                path,
                steps,
            } => Self::AdjustSplit {
                surface,
                path,
                steps,
            },
            DockAction::KeyboardAdjust(steps) => Self::KeyboardAdjust(steps),
            DockAction::BlurSplit => Self::BlurSplit,
            DockAction::ResetSplit { surface, path } => Self::ResetSplit { surface, path },
            DockAction::SurfaceResized {
                surface,
                width,
                height,
            } => Self::SurfaceResized {
                surface,
                width,
                height,
            },
            DockAction::SurfaceGeometry {
                surface,
                bounds,
                monitor,
            } => Self::SurfaceGeometry {
                surface,
                bounds,
                monitor,
            },
            DockAction::SurfaceLayout { surface, bounds } => {
                Self::SurfaceLayout { surface, bounds }
            }
            DockAction::DragStart { surface, id } => Self::DragStart { surface, id },
            DockAction::DragMove { surface, position } => Self::DragMove {
                surface,
                position: LogicalPoint::new(position.x, position.y),
            },
            DockAction::DragEnd { surface } => Self::DragEnd { surface },
            DockAction::CancelDrag => Self::CancelDrag,
            DockAction::AdvanceDragDwell => Self::AdvanceDragDwell,
            DockAction::Hover(_) => Self::AdvanceDragDwell,
            DockAction::CardHover(id, hovered) => Self::CardHover(id, hovered),
            DockAction::Hide(id) => Self::Hide(id),
            DockAction::Show(id) => Self::Show(id),
            DockAction::Float {
                id,
                bounds,
                monitor,
            } => Self::Float {
                id,
                bounds,
                monitor,
            },
            DockAction::Dock { id, target } => Self::Dock { id, target },
            DockAction::Focus(id) => Self::Focus(id),
            DockAction::CloseSurface(surface) => Self::CloseSurface(surface),
            DockAction::SetLocked(locked) => Self::SetLocked(locked),
            DockAction::Reset => Self::Reset,
        }
    }
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
    /// The persisted [`DockLayout`] changed and consumers should save it.
    /// Transient pointer, preview, focus and measured geometry updates remain false.
    pub changed: bool,
    pub effects: Vec<DockHostEffect>,
}

/// Quiet-period throttle for writing [`DockController::layout_json`].
///
/// Call [`Self::note`] with [`DockUpdate::changed`], [`Self::poll`] after the
/// quiet period, and [`Self::flush`] on blur or exit.
#[derive(Debug, Clone)]
pub struct DockLayoutPersist {
    delay: Duration,
    dirty: bool,
    last_change: Option<Duration>,
}

impl DockLayoutPersist {
    pub const DEFAULT_DELAY: Duration = Duration::from_millis(200);

    pub fn new() -> Self {
        Self::with_delay(Self::DEFAULT_DELAY)
    }

    pub fn with_delay(delay: Duration) -> Self {
        Self {
            delay,
            dirty: false,
            last_change: None,
        }
    }

    pub fn note(&mut self, changed: bool, now: Duration) {
        if !changed {
            return;
        }
        self.dirty = true;
        self.last_change = Some(now);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn poll(&mut self, now: Duration) -> bool {
        if !self.dirty || !self.last_change.is_some_and(|at| now >= at + self.delay) {
            return false;
        }
        self.clear();
        true
    }

    pub fn next_wakeup(&self) -> Option<Duration> {
        self.last_change
            .filter(|_| self.dirty)
            .map(|at| at + self.delay)
    }

    pub fn flush(&mut self) -> bool {
        let dirty = self.dirty;
        self.clear();
        dirty
    }

    fn clear(&mut self) {
        self.dirty = false;
        self.last_change = None;
    }
}

impl Default for DockLayoutPersist {
    fn default() -> Self {
        Self::new()
    }
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



pub(super) fn clamp_ratio(ratio: f32) -> f32 {
    finite(ratio, 0.5).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
}

pub(super) fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

pub(super) fn finite_positive(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}
