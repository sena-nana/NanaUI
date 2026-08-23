//! Dock controller: mutation reduction and layout queries.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::geometry::TITLE_BAR_HEIGHT as WINDOW_TITLE_BAR_HEIGHT;
#[cfg(feature = "hosted")]
use crate::runtime_host::RuntimeProgramUpdate;
use nana_ui_core::LogicalPoint;
#[cfg(feature = "hosted")]
use nana_ui_platform::WindowEvent;
#[cfg(all(test, feature = "hosted"))]
use nana_ui_platform::WindowGeometry;
#[cfg(all(test, feature = "hosted"))]
use nana_ui_platform::{WindowCommand, WindowId};

#[cfg(feature = "hosted")]
use super::host::hosted_dock_update;
use super::model::*;
use super::view::*;

#[derive(Debug, Clone)]
struct ActiveResize {
    surface: DockSurfaceId,
    path: Vec<usize>,
    axis: DockAxis,
    start_position: Option<f32>,
    start_ratio: f32,
    ratio_per_pixel: f32,
}

impl ActiveResize {
    fn value(&mut self, position: LogicalPoint) -> Option<f32> {
        let position = match self.axis {
            DockAxis::Horizontal => position.x,
            DockAxis::Vertical => position.y,
        };
        if !position.is_finite() {
            return None;
        }
        let Some(start_position) = self.start_position else {
            self.start_position = Some(position);
            return None;
        };
        Some(self.start_ratio + (position - start_position) * self.ratio_per_pixel)
    }
}

#[derive(Debug, Clone)]
struct ActiveDrag {
    source_surface: DockSurfaceId,
    id: DockId,
    start: Option<LogicalPoint>,
    position: Option<LogicalPoint>,
    moved: bool,
    pending_target: Option<(DockDropTarget, Duration)>,
    target: Option<DockDropTarget>,
    hover_surface: Option<DockSurfaceId>,
    transient_surface: Option<DockSurfaceId>,
    transient_ready: bool,
    original_bounds: Option<DockBounds>,
    bounds: Option<DockBounds>,
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

    fn local_to_global(self, position: LogicalPoint) -> LogicalPoint {
        LogicalPoint::new(self.window.x + position.x, self.window.y + position.y)
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
    floating_window_title: String,
    hovered_card: Option<DockId>,
    clock_origin: Instant,
    display_work_areas: BTreeMap<String, DockBounds>,
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
            floating_window_title: String::new(),
            hovered_card: None,
            clock_origin: Instant::now(),
            display_work_areas: BTreeMap::new(),
        })
    }

    pub fn layout(&self) -> &DockLayout {
        &self.layout
    }

    pub fn set_chrome_style(&mut self, chrome_style: DockChromeStyle) {
        self.chrome_style = chrome_style;
    }

    /// Sets the neutral title shown when a floating window contains a split root.
    /// This presentation setting is not part of the serialized dock layout.
    pub fn set_floating_window_title(&mut self, title: impl Into<String>) {
        self.floating_window_title = title.into();
    }

    /// Live logical work areas used to infer [`FloatingDock::monitor`].
    pub fn set_display_work_areas(&mut self, monitor_work_areas: BTreeMap<String, DockBounds>) {
        self.display_work_areas = monitor_work_areas;
    }

    pub fn item(&self, id: &DockId) -> Option<&DockItemSpec> {
        self.specs.get(id)
    }

    /// Projects the current persisted dock tree into logical rectangles.
    ///
    /// This is the canonical geometry entry for retained Runtime consumers;
    /// applications do not need to duplicate split arithmetic or title/tab
    /// chrome offsets. Pointer interaction is fed back through
    /// [`DockMutation`] using the reported split paths and surface identity.
    pub fn surface_layout(&self, surface: DockSurfaceId) -> Option<DockSurfaceLayout> {
        let root = self.surface_root(surface)?;
        let bounds = self.surface_layout_bounds(surface);
        let mut layout = DockSurfaceLayout {
            surface,
            bounds,
            items: Vec::new(),
            tabs: Vec::new(),
            splits: Vec::new(),
        };
        let (root_bounds, root_chrome) = if surface == DockSurfaceId(0) {
            (bounds, None)
        } else if matches!(root, DockNode::Split { .. }) {
            (bounds_below_chrome(bounds, WINDOW_TITLE_BAR_HEIGHT), None)
        } else {
            (bounds, Some(WINDOW_TITLE_BAR_HEIGHT))
        };
        collect_surface_layout(
            root,
            root_bounds,
            &self.center,
            &mut Vec::new(),
            &mut layout,
            root_chrome,
        );
        Some(layout)
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

    #[cfg(test)]
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

    #[cfg(test)]
    fn preview_root_for(&self, surface: DockSurfaceId) -> Option<DockViewNode> {
        let drag = self.active_drag.as_ref()?;
        let mut root = DockViewNode::from(self.surface_root(surface)?);
        if !drag.moved {
            return Some(root);
        }
        if drag.source_surface == surface && root.contains(&drag.id) {
            root = remove_view_node(root, &drag.id)?;
        }
        if let Some(target) = drag
            .target
            .as_ref()
            .filter(|target| target.surface == surface && target.zone != DockDropZone::Tab)
        {
            let placeholder = DockViewItem::Placeholder(drag.id.clone());
            insert_view_node(&mut root, &target.id, placeholder, target.zone);
        }
        Some(root)
    }

    fn settle_drag_target(&self, drag: &mut ActiveDrag, now: Duration) {
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

    /// Restores JSON and clamps floating windows to logical work areas.
    pub fn restore_layout_json_clamped(
        &mut self,
        value: &str,
        monitor_work_areas: &BTreeMap<String, DockBounds>,
        primary_work_area: DockBounds,
    ) -> Result<DockUpdate, DockError> {
        let mut effects = self.restore_layout_json(value)?;
        let clamp = self.clamp_floating_bounds(monitor_work_areas, primary_work_area);
        for effect in &mut effects {
            if let DockHostEffect::OpenFloating(opened) = effect
                && let Some(current) = self
                    .layout
                    .floating
                    .iter()
                    .find(|floating| floating.surface == opened.surface)
            {
                *opened = current.clone();
            }
        }
        for effect in clamp.effects {
            if let DockHostEffect::MoveFloating { surface, .. } = &effect
                && effects.iter().any(|existing| {
                    matches!(
                        existing,
                        DockHostEffect::OpenFloating(opened) if opened.surface == *surface
                    )
                })
            {
                continue;
            }
            effects.push(effect);
        }
        Ok(DockUpdate {
            changed: clamp.changed,
            effects,
        })
    }

    /// Applies geometry and close events emitted by the Nana Scene host.
    #[cfg(feature = "hosted")]
    pub fn update_hosted_window(&mut self, event: WindowEvent) -> DockUpdate {
        match event {
            WindowEvent::Ready { id, geometry }
            | WindowEvent::Resized { id, geometry }
            | WindowEvent::Moved { id, geometry } => {
                let surface = DockSurfaceId::from(id);
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
                        geometry.logical_size.0,
                        geometry.logical_size.1,
                    ));
                let position = geometry
                    .logical_position
                    .map(|(x, y)| LogicalPoint::new(x, y))
                    .unwrap_or(LogicalPoint::new(previous.x, previous.y));
                let bounds = DockBounds::new(
                    position.x,
                    position.y,
                    geometry.logical_size.0,
                    geometry.logical_size.1,
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
                self.update(DockAction::SurfaceGeometry {
                    surface,
                    bounds,
                    monitor: resolve_monitor_id(&self.display_work_areas, bounds),
                })
            }
            WindowEvent::CloseRequested { id } if DockSurfaceId::from(id) != DockSurfaceId(0) => {
                self.update(DockAction::CloseSurface(DockSurfaceId::from(id)))
            }
            _ => DockUpdate::default(),
        }
    }

    /// Opens every floating surface in the current layout.
    #[cfg(feature = "hosted")]
    pub fn open_hosted_windows(&self, title: impl Into<String>) -> RuntimeProgramUpdate {
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

    /// Clamps restored floating bounds, then opens those windows.
    #[cfg(feature = "hosted")]
    pub fn open_restored_hosted_windows(
        &mut self,
        title: impl Into<String>,
        monitor_work_areas: &BTreeMap<String, DockBounds>,
        primary_work_area: DockBounds,
    ) -> (RuntimeProgramUpdate, bool) {
        let clamp = self.clamp_floating_bounds(monitor_work_areas, primary_work_area);
        (self.open_hosted_windows(title), clamp.changed)
    }

    /// Clamps floating windows to logical work areas; missing monitors use primary.
    pub fn clamp_floating_bounds(
        &mut self,
        monitor_work_areas: &BTreeMap<String, DockBounds>,
        primary_work_area: DockBounds,
    ) -> DockUpdate {
        self.display_work_areas = monitor_work_areas.clone();
        let mut changed = false;
        let mut effects = Vec::new();
        for floating in &mut self.layout.floating {
            let named_missing = floating
                .monitor
                .as_ref()
                .is_some_and(|monitor| !monitor_work_areas.contains_key(monitor));
            let work_area = floating
                .monitor
                .as_ref()
                .and_then(|monitor| monitor_work_areas.get(monitor))
                .copied()
                .unwrap_or(primary_work_area);
            if named_missing {
                floating.monitor = None;
                changed = true;
            }
            let bounds = floating.bounds.clamped_to(work_area);
            let moved = bounds != floating.bounds;
            changed |= moved;
            floating.bounds = bounds;
            let surface = floating.surface;
            self.surface_geometry
                .entry(surface)
                .or_insert(DockSurfaceGeometry::new(bounds))
                .set_window(bounds);
            if moved {
                effects.push(DockHostEffect::MoveFloating { surface, bounds });
            }
        }
        DockUpdate { changed, effects }
    }

    pub fn update(&mut self, action: DockAction) -> DockUpdate {
        self.update_mutation(action.into())
    }

    /// Applies one backend-neutral mutation using the controller's monotonic clock.
    pub fn update_mutation(&mut self, mutation: DockMutation) -> DockUpdate {
        self.update_mutation_at(mutation, self.clock_origin.elapsed())
    }

    #[cfg(test)]
    fn update_at(&mut self, action: DockAction, now: Instant) -> DockUpdate {
        self.update_at_duration(action, now.saturating_duration_since(self.clock_origin))
    }

    /// Compatibility action entry at an explicit monotonic timestamp.
    pub fn update_at_duration(&mut self, action: DockAction, now: Duration) -> DockUpdate {
        self.update_mutation_at(action.into(), now)
    }

    /// Applies one backend-neutral mutation at an explicit monotonic timestamp.
    pub fn update_mutation_at(&mut self, action: DockMutation, now: Duration) -> DockUpdate {
        use DockMutation as DockAction;

        if self.layout.locked
            && !matches!(
                action,
                DockAction::SetLocked(_)
                    | DockAction::Focus(_)
                    | DockAction::ActivateTab(_)
                    | DockAction::SurfaceResized { .. }
                    | DockAction::SurfaceGeometry { .. }
                    | DockAction::SurfaceLayout { .. }
                    | DockAction::CardHover(..)
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
                    axis,
                    start_position: None,
                    start_ratio: ratio,
                    ratio_per_pixel: 1.0 / extent,
                });
                self.focused_split = geometry.map(|(axis, _, _)| (surface, path, axis));
                DockUpdate::default()
            }
            DockAction::ResizeMove(position) => {
                let Some(active) = &mut self.active_resize else {
                    return DockUpdate::default();
                };
                let Some(ratio) = active.value(position) else {
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
                self.update_mutation_at(
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
                geometry.set_layout(layout);
                if is_drag_preview {
                    return DockUpdate::default();
                }
                let mut changed = false;
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
            DockAction::SurfaceGeometry {
                surface,
                bounds,
                monitor,
            } => {
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
                let inferred = resolve_monitor_id(&self.display_work_areas, bounds);
                let mut changed = false;
                if let Some(floating) = self
                    .layout
                    .floating
                    .iter_mut()
                    .find(|floating| floating.surface == surface)
                {
                    changed |= floating.bounds != bounds;
                    floating.bounds = bounds;
                    if let Some(next_monitor) = monitor.or(inferred) {
                        changed |= floating.monitor.as_ref() != Some(&next_monitor);
                        floating.monitor = Some(next_monitor);
                    }
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
                    source_surface: surface,
                    id,
                    start: None,
                    position: None,
                    moved: false,
                    pending_target: None,
                    target: None,
                    hover_surface: None,
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
                drag.hover_surface = self.hover_surface_at(&drag, surface, position);
                let next_candidate = drag
                    .moved
                    .then(|| {
                        drag.hover_surface.and_then(|hover_surface| {
                            self.drop_target_at(
                                &drag.id,
                                position,
                                drag.source_surface,
                                hover_surface,
                            )
                        })
                    })
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
            DockAction::AdvanceDragDwell => {
                let Some(mut drag) = self.active_drag.take() else {
                    return DockUpdate::default();
                };
                self.settle_drag_target(&mut drag, now);
                self.active_drag = Some(drag);
                DockUpdate::default()
            }
            DockAction::CardHover(id, hovered) => {
                if hovered {
                    self.hovered_card = Some(id);
                } else if self.hovered_card.as_ref() == Some(&id) {
                    self.hovered_card = None;
                }
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

    fn drag_card_bounds(position: LogicalPoint) -> DockBounds {
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
        position: LogicalPoint,
    ) -> Option<DockHostEffect> {
        if drag.transient_surface.is_some() {
            return None;
        }
        if drag.source_surface == DockSurfaceId(0) {
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
            let bounds = self.surface_window_bounds(drag.source_surface);
            drag.original_bounds = Some(bounds);
            drag.bounds = Some(bounds);
            let reuse_source = self
                .surface_root(drag.source_surface)
                .is_some_and(|root| matches!(root, DockNode::Item { id } if id == &drag.id));
            if reuse_source {
                drag.transient_surface = Some(drag.source_surface);
                drag.transient_ready = true;
                None
            } else {
                let surface = DockSurfaceId(self.next_surface);
                self.next_surface = self.next_surface.saturating_add(1);
                let monitor = self
                    .layout
                    .floating
                    .iter()
                    .find(|floating| floating.surface == drag.source_surface)
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

    fn drag_bounds(&self, drag: &ActiveDrag, position: LogicalPoint) -> Option<DockBounds> {
        let current = drag.bounds?;
        let bounds = if drag.source_surface == DockSurfaceId(0) {
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
        if drag.source_surface == DockSurfaceId(0) {
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
        } else if drag.transient_surface == Some(drag.source_surface) {
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
                .find(|floating| floating.surface == drag.source_surface)
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
        if drag.source_surface == DockSurfaceId(0)
            || drag.transient_surface != Some(drag.source_surface)
        {
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

    #[cfg(test)]
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
        position: LogicalPoint,
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

    fn hover_surface_at(
        &self,
        drag: &ActiveDrag,
        event_surface: DockSurfaceId,
        position: LogicalPoint,
    ) -> Option<DockSurfaceId> {
        let transient_surface = drag.transient_surface;
        let contains = |surface| {
            Some(surface) != transient_surface
                && bounds_contains(self.surface_window_bounds(surface), position)
        };
        if contains(event_surface) {
            return Some(event_surface);
        }
        if let Some(surface) = drag.hover_surface.filter(|surface| contains(*surface)) {
            return Some(surface);
        }
        self.layout
            .floating
            .iter()
            .rev()
            .map(|floating| floating.surface)
            .find(|surface| contains(*surface))
            .or_else(|| contains(DockSurfaceId(0)).then_some(DockSurfaceId(0)))
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

    fn local_to_global(&self, surface: DockSurfaceId, position: LogicalPoint) -> LogicalPoint {
        self.surface_geometry
            .get(&surface)
            .copied()
            .unwrap_or_else(|| DockSurfaceGeometry::new(self.surface_window_bounds(surface)))
            .local_to_global(position)
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

fn resolve_monitor_id(
    monitor_work_areas: &BTreeMap<String, DockBounds>,
    bounds: DockBounds,
) -> Option<String> {
    monitor_work_areas
        .iter()
        .filter_map(|(id, area)| {
            let area = bounds.intersection_area(*area);
            (area > 0.0).then_some((area, id.clone()))
        })
        .max_by(|(left, left_id), (right, right_id)| {
            left.partial_cmp(right)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left_id.cmp(right_id))
        })
        .map(|(_, id)| id)
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

fn collect_surface_layout(
    node: &DockNode,
    bounds: DockBounds,
    center: &DockId,
    path: &mut Vec<usize>,
    output: &mut DockSurfaceLayout,
    chrome_override: Option<f32>,
) {
    match node {
        DockNode::Item { id } => {
            let content = if id == center {
                bounds
            } else {
                bounds_below_chrome(bounds, chrome_override.unwrap_or(TITLE_BAR_HEIGHT))
            };
            output.items.push(DockItemLayout {
                id: id.clone(),
                panel: bounds,
                content,
            });
        }
        DockNode::Tabs { tabs, active } => {
            let content = bounds_below_chrome(bounds, chrome_override.unwrap_or(TITLE_BAR_HEIGHT));
            output.tabs.push(DockTabsLayout {
                tabs: tabs.clone(),
                active: active.clone(),
                bounds,
                content,
            });
            output.items.push(DockItemLayout {
                id: active.clone(),
                panel: bounds,
                content,
            });
        }
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let (first_bounds, second_bounds) = split_child_bounds(*axis, *ratio, bounds);
            let splitter = match axis {
                DockAxis::Horizontal => DockBounds::new(
                    first_bounds.x + first_bounds.width,
                    bounds.y,
                    DIVIDER_HIT_SIZE.min(bounds.width),
                    bounds.height,
                ),
                DockAxis::Vertical => DockBounds::new(
                    bounds.x,
                    first_bounds.y + first_bounds.height,
                    bounds.width,
                    DIVIDER_HIT_SIZE.min(bounds.height),
                ),
            };
            output.splits.push(DockSplitLayout {
                path: path.clone(),
                axis: *axis,
                bounds: splitter,
            });
            path.push(0);
            collect_surface_layout(first, first_bounds, center, path, output, None);
            path.pop();
            path.push(1);
            collect_surface_layout(second, second_bounds, center, path, output, None);
            path.pop();
        }
    }
}

fn bounds_below_chrome(bounds: DockBounds, chrome_height: f32) -> DockBounds {
    let chrome = chrome_height.min(bounds.height);
    DockBounds::new(
        bounds.x,
        bounds.y + chrome,
        bounds.width,
        bounds.height - chrome,
    )
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

#[cfg(test)]
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

#[cfg(test)]
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

fn bounds_contains(bounds: DockBounds, point: LogicalPoint) -> bool {
    point.x >= bounds.x
        && point.y >= bounds.y
        && point.x <= bounds.x + bounds.width
        && point.y <= bounds.y + bounds.height
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

#[cfg(test)]
fn node_contains_drop_target(node: &DockViewNode, id: &DockId) -> bool {
    match node {
        DockViewNode::Item { item } => item.id() == id,
        DockViewNode::Tabs { tabs, .. } => tabs.iter().any(|item| item.id() == id),
        DockViewNode::Split { .. } => false,
    }
}

#[cfg(test)]
fn node_is_drop_highlighted(
    controller: &DockController,
    surface: DockSurfaceId,
    node: &DockViewNode,
) -> bool {
    let Some(drag) = controller.active_drag.as_ref() else {
        return false;
    };
    let Some(target) = controller
        .drop_highlight_target()
        .filter(|target| target.surface == surface)
    else {
        return false;
    };
    if target.zone != DockDropZone::Tab
        && drag
            .target
            .as_ref()
            .is_some_and(|settled| settled == target)
    {
        node.contains_placeholder(&drag.id)
    } else {
        node_contains_drop_target(node, &target.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            source_surface: DockSurfaceId(0),
            id: DockId::from("source"),
            start: None,
            position: None,
            moved: true,
            pending_target: None,
            target: Some(target),
            hover_surface: Some(DockSurfaceId(0)),
            transient_surface: None,
            transient_ready: false,
            original_bounds: None,
            bounds: None,
        });
        controller
    }

    #[test]
    fn surface_layout_projects_split_tabs_and_application_content_bounds() {
        let mut controller = controller();
        let layout = controller
            .surface_layout(DockSurfaceId(0))
            .expect("main surface layout");

        assert_eq!(layout.bounds, DockBounds::new(0.0, 0.0, 1280.0, 800.0));
        assert_eq!(
            layout.splits,
            vec![
                DockSplitLayout {
                    path: vec![],
                    axis: DockAxis::Horizontal,
                    bounds: DockBounds::new(318.0, 0.0, 8.0, 800.0),
                },
                DockSplitLayout {
                    path: vec![1],
                    axis: DockAxis::Vertical,
                    bounds: DockBounds::new(326.0, 594.0, 954.0, 8.0),
                },
            ]
        );
        assert_eq!(layout.tabs.len(), 2);
        assert_eq!(layout.tabs[0].active, DockId::from("scenes"));
        assert_eq!(
            layout.tabs[0].content,
            DockBounds::new(0.0, 28.0, 318.0, 772.0)
        );
        assert_eq!(layout.tabs[1].active, DockId::from("mixer"));
        assert_eq!(
            layout.tabs[1].content,
            DockBounds::new(326.0, 630.0, 954.0, 170.0)
        );
        assert_eq!(
            layout
                .items
                .iter()
                .find(|item| item.id == DockId::from("editor"))
                .expect("editor")
                .content,
            DockBounds::new(326.0, 0.0, 954.0, 594.0)
        );
        assert!(
            layout
                .items
                .iter()
                .all(|item| item.id != DockId::from("sources"))
        );

        assert!(
            controller
                .update_mutation(DockMutation::ActivateTab(DockId::from("sources")))
                .changed
        );
        let updated = controller
            .surface_layout(DockSurfaceId(0))
            .expect("updated main surface layout");
        assert_eq!(updated.tabs[0].active, DockId::from("sources"));
        assert!(
            updated
                .items
                .iter()
                .any(|item| item.id == DockId::from("sources"))
        );
        assert!(
            updated
                .items
                .iter()
                .all(|item| item.id != DockId::from("scenes"))
        );
    }

    #[test]
    fn floating_surface_layout_reserves_native_title_bar_once() {
        let layout = DockLayout {
            version: DOCK_LAYOUT_VERSION,
            main: DockNode::item("editor"),
            floating: vec![
                FloatingDock {
                    surface: DockSurfaceId(1),
                    root: DockNode::item("scenes"),
                    bounds: DockBounds::new(20.0, 30.0, 400.0, 300.0),
                    monitor: None,
                },
                FloatingDock {
                    surface: DockSurfaceId(2),
                    root: DockNode::split(
                        DockAxis::Horizontal,
                        0.5,
                        DockNode::item("sources"),
                        DockNode::item("mixer"),
                    ),
                    bounds: DockBounds::new(40.0, 50.0, 500.0, 320.0),
                    monitor: None,
                },
            ],
            hidden: Vec::new(),
            locked: false,
        };
        let controller = DockController::new(
            "editor",
            [
                DockItemSpec::new("editor", "Editor"),
                DockItemSpec::new("scenes", "Scenes"),
                DockItemSpec::new("sources", "Sources"),
                DockItemSpec::new("mixer", "Mixer"),
            ],
            layout,
        )
        .expect("valid floating layout");

        let item = controller
            .surface_layout(DockSurfaceId(1))
            .expect("floating item");
        assert_eq!(
            item.items[0].content,
            DockBounds::new(0.0, 36.0, 400.0, 264.0)
        );

        let split = controller
            .surface_layout(DockSurfaceId(2))
            .expect("floating split");
        assert_eq!(
            split.splits[0].bounds,
            DockBounds::new(246.0, 36.0, 8.0, 284.0)
        );
        assert_eq!(
            split.items[0].content,
            DockBounds::new(0.0, 64.0, 246.0, 256.0)
        );
        assert_eq!(
            split.items[1].content,
            DockBounds::new(254.0, 64.0, 246.0, 256.0)
        );
    }

    #[test]
    fn drop_highlight_targets_the_complete_item_or_tabs_card() {
        let item = DockViewNode::Item {
            item: DockViewItem::Existing(DockId::from("editor")),
        };
        let tabs = DockViewNode::Tabs {
            tabs: vec![
                DockViewItem::Existing(DockId::from("scenes")),
                DockViewItem::Existing(DockId::from("editor")),
            ],
            active: DockViewItem::Existing(DockId::from("editor")),
        };
        let unrelated = DockViewNode::Item {
            item: DockViewItem::Existing(DockId::from("sources")),
        };
        let split = DockViewNode::Split {
            axis: DockAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(item.clone()),
            second: Box::new(unrelated.clone()),
        };

        let tab_controller = preview_controller(DockDropZone::Tab);
        assert!(node_is_drop_highlighted(
            &tab_controller,
            DockSurfaceId(0),
            &item
        ));
        assert!(node_is_drop_highlighted(
            &tab_controller,
            DockSurfaceId(0),
            &tabs
        ));
        assert!(!node_is_drop_highlighted(
            &tab_controller,
            DockSurfaceId(0),
            &unrelated
        ));
        assert!(!node_is_drop_highlighted(
            &tab_controller,
            DockSurfaceId(0),
            &split
        ));

        let split_controller = preview_controller(DockDropZone::Left);
        let split_preview = split_controller.preview_root().expect("split preview");
        let DockViewNode::Split { first, second, .. } = split_preview else {
            panic!("split preview expected")
        };
        assert!(node_is_drop_highlighted(
            &split_controller,
            DockSurfaceId(0),
            &first
        ));
        assert!(!node_is_drop_highlighted(
            &split_controller,
            DockSurfaceId(0),
            &second
        ));
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
    fn drag_preview_tab_keeps_existing_target_without_placeholder() {
        let controller = preview_controller(DockDropZone::Tab);
        let preview = controller.preview_root().expect("tab preview");
        assert_eq!(
            preview,
            DockViewNode::Item {
                item: DockViewItem::Existing(DockId::from("editor")),
            }
        );
        assert!(!contains_placeholder(&preview));
        assert!(node_is_drop_highlighted(
            &controller,
            DockSurfaceId(0),
            &preview
        ));
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
        let now = Instant::now();
        assert!(!controller.is_drag_frame_needed());

        controller.update_at(
            DockAction::DragStart {
                surface: DockSurfaceId(0),
                id: DockId::from("source"),
            },
            now,
        );
        assert!(!controller.is_drag_frame_needed());

        move_source_to_position(&mut controller, now, LogicalPoint::new(50.0, 400.0));
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
            position: LogicalPoint::new(0.0, 0.0),
        });
        let update = controller.update(DockAction::DragMove {
            surface: DockSurfaceId(0),
            position: LogicalPoint::new(100.0, 400.0),
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
        now: Instant,
        position: LogicalPoint,
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
                position: LogicalPoint::new(0.0, 0.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position,
            },
            now + Duration::from_millis(1),
        )
    }

    const DRAG_TEST_TICK: Duration = Duration::from_millis(1);

    fn after_drag_dwell(at: Instant) -> Instant {
        at + DRAG_INSERT_HOVER_DELAY + DRAG_TEST_TICK
    }

    #[test]
    fn real_tab_drop_commits_the_preview_layout() {
        let mut controller = tab_drag_controller();
        let now = Instant::now();
        let opened = move_source_to_position(&mut controller, now, LogicalPoint::new(300.0, 400.0));
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
        let now = Instant::now();
        move_source_to_position(&mut controller, now, LogicalPoint::new(50.0, 400.0));
        let settled = controller.update_at(DockAction::AdvanceDragDwell, after_drag_dwell(now));
        assert!(!settled.changed, "preview state is not persisted layout");
        assert_eq!(
            controller.drop_target().map(|target| target.zone),
            Some(DockDropZone::Left)
        );

        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(300.0, 400.0),
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
        let DockViewNode::Split { ref first, .. } = tab_preview else {
            panic!("target split should remain in the preview root")
        };
        let DockViewNode::Item { item } = first.as_ref() else {
            panic!("tab target should remain an existing item")
        };
        assert_eq!(item, &DockViewItem::Existing(DockId::from("target")));
        assert!(!contains_placeholder(&tab_preview));
        assert!(node_is_drop_highlighted(
            &controller,
            DockSurfaceId(0),
            first.as_ref()
        ));
    }

    #[test]
    fn rapid_candidate_changes_commit_only_the_latest_preview_target() {
        let mut controller = tab_drag_controller();
        let before = controller.layout().clone();
        let before_json = controller.layout_json().expect("layout json");
        let now = Instant::now();

        let opened = move_source_to_position(&mut controller, now, LogicalPoint::new(50.0, 400.0));
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
                position: LogicalPoint::new(300.0, 400.0),
            },
            retargeted_at,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(630.0, 400.0),
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
        let now = Instant::now();
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
                position: LogicalPoint::new(100.0, 100.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: target,
                position: LogicalPoint::new(180.0, 140.0),
            },
            now + Duration::from_millis(1),
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
                position: LogicalPoint::new(300.0, 400.0),
            },
            retargeted_at,
        );
        let latest_target_at = retargeted_at + DRAG_TEST_TICK;
        controller.update_at(
            DockAction::DragMove {
                surface: target,
                position: LogicalPoint::new(180.0, 140.0),
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
        assert_eq!(
            target_surface,
            DockViewNode::Item {
                item: DockViewItem::Existing(DockId::from("mixer")),
            }
        );
        assert!(!contains_placeholder(&target_surface));
        assert!(node_is_drop_highlighted(
            &controller,
            target,
            &target_surface
        ));
    }

    #[test]
    fn leaving_and_reentering_before_dwell_only_settles_the_reentered_target() {
        let mut controller = simple_drag_controller();
        let before = controller.layout().clone();
        let before_json = controller.layout_json().expect("layout json");
        let now = Instant::now();
        move_source_to_position(&mut controller, now, LogicalPoint::new(100.0, 400.0));
        assert!(controller.drop_target().is_none());

        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(700.0, 400.0),
            },
            now + Duration::from_millis(100),
        );
        assert!(controller.drop_highlight_target().is_none());
        assert!(!contains_placeholder(
            &controller.preview_root().expect("drag preview")
        ));

        let reentered_at = now + Duration::from_millis(151);
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(100.0, 400.0),
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
        let now = Instant::now();
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
                position: LogicalPoint::new(0.0, 0.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(2.0, 2.0),
            },
            now + Duration::from_millis(1),
        );
        assert!(controller.drop_target().is_none());
        assert!(controller.drop_highlight_target().is_none());

        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(100.0, 400.0),
            },
            now + Duration::from_millis(2),
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
            now + DRAG_INSERT_HOVER_DELAY + Duration::from_millis(1),
        );
        assert!(controller.drop_target().is_none());
        controller.update_at(
            DockAction::Hover(false),
            now + DRAG_INSERT_HOVER_DELAY + Duration::from_millis(2),
        );
        assert!(controller.drop_target().is_some());
    }

    #[test]
    fn changing_or_leaving_a_candidate_resets_or_cancels_the_dwell() {
        let mut controller = simple_drag_controller();
        let before = controller.layout().clone();
        let now = Instant::now();
        move_source_to_position(&mut controller, now, LogicalPoint::new(100.0, 400.0));
        assert!(controller.drop_target().is_none());

        let changed_at = now + DRAG_INSERT_HOVER_DELAY;
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(1200.0, 400.0),
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

        let left_at = now + Duration::from_millis(600);
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(700.0, 400.0),
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
        let now = Instant::now();
        let mut before_deadline = simple_drag_controller();
        let opened =
            move_source_to_position(&mut before_deadline, now, LogicalPoint::new(100.0, 400.0));
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
        let opened =
            move_source_to_position(&mut at_deadline, now, LogicalPoint::new(100.0, 400.0));
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
        let now = Instant::now();
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
                position: LogicalPoint::new(0.0, 0.0),
            },
            now,
        );
        let opened = controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(100.0, 400.0),
            },
            now + Duration::from_millis(1),
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
            preview_ready_at + Duration::from_millis(1),
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
            position: LogicalPoint::new(0.0, 0.0),
        });
        let opened = controller.update(DockAction::DragMove {
            surface: DockSurfaceId(0),
            position: LogicalPoint::new(700.0, 400.0),
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
            position: LogicalPoint::new(100.0, 100.0),
        });
        controller.update(DockAction::DragMove {
            surface: floating.surface,
            position: LogicalPoint::new(-600.0, -600.0),
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
            monitor: None,
        });

        let now = Instant::now();
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
                position: LogicalPoint::new(10.0, 10.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(150.0, 250.0),
            },
            now + Duration::from_millis(1),
        );
        let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        let update = controller.update_at(
            DockAction::DragEnd {
                surface: DockSurfaceId(0),
            },
            preview_ready_at + Duration::from_millis(1),
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
        let now = Instant::now();
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
                position: LogicalPoint::new(100.0, 100.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: target,
                position: LogicalPoint::new(180.0, 140.0),
            },
            now + Duration::from_millis(1),
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
            preview_ready_at + Duration::from_millis(1),
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
            (DockDropZone::Left, LogicalPoint::new(20.0, 140.0), true),
            (DockDropZone::Right, LogicalPoint::new(340.0, 140.0), false),
            (DockDropZone::Top, LogicalPoint::new(180.0, 20.0), true),
            (DockDropZone::Bottom, LogicalPoint::new(180.0, 260.0), false),
        ];
        for (zone, target_position, inserted_first) in cases {
            let (mut controller, source, target) = floating_pair_controller();
            let now = Instant::now();
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
                    position: LogicalPoint::new(100.0, 100.0),
                },
                now,
            );
            controller.update_at(
                DockAction::DragMove {
                    surface: target,
                    position: target_position,
                },
                now + Duration::from_millis(1),
            );
            let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
            controller.update_at(DockAction::Hover(false), preview_ready_at);
            assert_eq!(
                controller.drop_target().map(|target| target.zone),
                Some(zone)
            );
            let update = controller.update_at(
                DockAction::DragEnd { surface: target },
                preview_ready_at + Duration::from_millis(1),
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
        let now = Instant::now();
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
                position: LogicalPoint::new(100.0, 100.0),
            },
            now,
        );
        let opened = controller.update_at(
            DockAction::DragMove {
                surface: target,
                position: LogicalPoint::new(180.0, 140.0),
            },
            now + Duration::from_millis(1),
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
            preview_ready_at + Duration::from_millis(1),
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
        let now = Instant::now();
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
                position: LogicalPoint::new(100.0, 100.0),
            },
            now,
        );
        let opened = cancelled.update_at(
            DockAction::DragMove {
                surface: source,
                position: LogicalPoint::new(108.0, 108.0),
            },
            now + Duration::from_millis(1),
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
        let now = Instant::now();
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
                position: LogicalPoint::new(100.0, 100.0),
            },
            now,
        );
        let opened = released.update_at(
            DockAction::DragMove {
                surface: source,
                position: LogicalPoint::new(700.0, 500.0),
            },
            now + Duration::from_millis(1),
        );
        let DockHostEffect::OpenFloating(transient) = &opened.effects[0] else {
            panic!("grouped drag opens a transient panel surface")
        };
        let update = released.update_at(
            DockAction::DragEnd { surface: source },
            now + Duration::from_millis(2),
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
        let update = controller
            .clamp_floating_bounds(&BTreeMap::new(), DockBounds::new(0.0, 0.0, 1280.0, 900.0));
        assert!(update.changed);
        let floating = &controller.layout().floating[0];
        assert_eq!(floating.monitor, None);
        assert_eq!(floating.bounds, DockBounds::new(0.0, 0.0, 1280.0, 900.0));
        assert_eq!(
            update.effects,
            vec![DockHostEffect::MoveFloating {
                surface: floating.surface,
                bounds: floating.bounds,
            }]
        );
    }

    #[test]
    fn move_updates_layout_json_immediately_and_round_trips() {
        let mut source = controller();
        source.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(40.0, 50.0, 360.0, 280.0),
            monitor: None,
        });
        let surface = source.layout().floating[0].surface;
        let initial = source.layout().floating[0].bounds;
        let moved = DockBounds::new(120.0, 80.0, 360.0, 280.0);
        let update = source.update(DockAction::SurfaceGeometry {
            surface,
            bounds: moved,
            monitor: None,
        });
        assert!(update.changed);
        assert_eq!(source.layout().floating[0].bounds, moved);
        assert_ne!(source.layout().floating[0].bounds, initial);
        let encoded = source.layout_json().expect("layout json");
        let snapshot: DockLayout = serde_json::from_str(&encoded).expect("layout parses");
        assert_eq!(snapshot.floating[0].bounds, moved);

        let mut restored = controller();
        restored
            .restore_layout_json(&encoded)
            .expect("geometry restores");
        assert_eq!(restored.layout().floating[0].bounds, moved);
    }

    #[test]
    fn resize_keeps_origin_and_round_trips_size() {
        let mut source = controller();
        source.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(40.0, 50.0, 360.0, 280.0),
            monitor: None,
        });
        let surface = source.layout().floating[0].surface;
        let update = source.update(DockAction::SurfaceResized {
            surface,
            width: 420.0,
            height: 320.0,
        });
        assert!(update.changed);
        assert_eq!(
            source.layout().floating[0].bounds,
            DockBounds::new(40.0, 50.0, 420.0, 320.0)
        );
        let encoded = source.layout_json().expect("layout json");
        let mut restored = controller();
        restored
            .restore_layout_json(&encoded)
            .expect("resized geometry restores");
        assert_eq!(
            restored.layout().floating[0].bounds,
            DockBounds::new(40.0, 50.0, 420.0, 320.0)
        );
    }

    #[test]
    fn main_surface_geometry_does_not_enter_floating_layout() {
        let mut controller = controller();
        assert!(controller.layout().floating.is_empty());
        let update = controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(10.0, 20.0, 1_000.0, 700.0),
            monitor: None,
        });
        assert!(!update.changed);
        assert!(controller.layout().floating.is_empty());
        let encoded = controller.layout_json().expect("layout json");
        assert!(!encoded.contains("\"floating\":[{"));
    }

    #[test]
    fn restoring_on_missing_monitor_clamps_before_open_and_persists() {
        let mut source = controller();
        source.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(4_000.0, -500.0, 2_000.0, 1_500.0),
            monitor: Some("gone".into()),
        });
        let encoded = source.layout_json().expect("layout json");
        let primary = DockBounds::new(0.0, 0.0, 1280.0, 800.0);
        let mut restored = controller();
        let update = restored
            .restore_layout_json_clamped(&encoded, &BTreeMap::new(), primary)
            .expect("clamped restore");
        assert!(update.changed);
        let DockHostEffect::OpenFloating(opened) = &update.effects[0] else {
            panic!("restored floating open");
        };
        assert_eq!(opened.bounds, DockBounds::new(0.0, 0.0, 1280.0, 800.0));
        assert_eq!(opened.monitor, None);
        let corrected = restored.layout_json().expect("corrected json");
        let mut again = controller();
        let second = again
            .restore_layout_json_clamped(&corrected, &BTreeMap::new(), primary)
            .expect("second restore");
        assert!(!second.changed);
        assert_eq!(
            again.layout().floating[0].bounds,
            DockBounds::new(0.0, 0.0, 1280.0, 800.0)
        );
    }

    #[test]
    fn smaller_logical_work_area_clamps_position_and_size() {
        let work_area = DockBounds::new(0.0, 0.0, 1280.0, 720.0);
        let original = DockBounds::new(1_000.0, 40.0, 2_000.0, 1_500.0);
        assert_eq!(
            original.clamped_to(work_area),
            DockBounds::new(0.0, 0.0, 1280.0, 720.0)
        );

        let mut controller = controller();
        controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: original,
            monitor: Some("built-in".into()),
        });
        let monitors = BTreeMap::from([("built-in".to_string(), work_area)]);
        let update = controller.clamp_floating_bounds(&monitors, work_area);
        assert!(update.changed);
        assert_eq!(
            controller.layout().floating[0].monitor.as_deref(),
            Some("built-in")
        );
        assert_eq!(
            controller.layout().floating[0].bounds,
            original.clamped_to(work_area)
        );
    }

    fn dual_monitor_work_areas() -> (BTreeMap<String, DockBounds>, DockBounds) {
        let primary = DockBounds::new(0.0, 0.0, 1280.0, 800.0);
        let monitors = BTreeMap::from([
            ("built-in".to_string(), primary),
            (
                "display-2".to_string(),
                DockBounds::new(1280.0, 0.0, 1920.0, 1080.0),
            ),
        ]);
        (monitors, primary)
    }

    #[test]
    fn geometry_persists_named_monitor_and_restore_stays_on_secondary() {
        let (monitors, primary) = dual_monitor_work_areas();
        let mut source = controller();
        source.set_display_work_areas(monitors.clone());
        source.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(1_400.0, 80.0, 360.0, 280.0),
            monitor: Some("built-in".into()),
        });
        let surface = source.layout().floating[0].surface;
        let moved = DockBounds::new(1_600.0, 120.0, 400.0, 300.0);
        let update = source.update(DockAction::SurfaceGeometry {
            surface,
            bounds: moved,
            monitor: None,
        });
        assert!(update.changed);
        assert_eq!(source.layout().floating[0].bounds, moved);
        assert_eq!(
            source.layout().floating[0].monitor.as_deref(),
            Some("display-2")
        );

        let encoded = source.layout_json().expect("layout json");
        let snapshot: DockLayout = serde_json::from_str(&encoded).expect("layout parses");
        assert_eq!(snapshot.floating[0].monitor.as_deref(), Some("display-2"));

        let mut restored = controller();
        let first = restored
            .restore_layout_json_clamped(&encoded, &monitors, primary)
            .expect("secondary restore");
        assert!(!first.changed);
        assert_eq!(restored.layout().floating[0].bounds, moved);
        assert_eq!(
            restored.layout().floating[0].monitor.as_deref(),
            Some("display-2")
        );

        let corrected = restored.layout_json().expect("stable json");
        let mut again = controller();
        let second = again
            .restore_layout_json_clamped(&corrected, &monitors, primary)
            .expect("second restore");
        assert!(!second.changed);
        assert_eq!(again.layout().floating[0].bounds, moved);
        assert_eq!(
            again.layout().floating[0].monitor.as_deref(),
            Some("display-2")
        );
    }

    #[test]
    fn unplugging_secondary_clamps_only_that_window_then_stops_drifting() {
        let (_, primary) = dual_monitor_work_areas();
        let mut source = controller();
        source.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(1_600.0, 80.0, 400.0, 300.0),
            monitor: Some("display-2".into()),
        });
        source.update(DockAction::Float {
            id: "mixer".into(),
            bounds: DockBounds::new(80.0, 60.0, 360.0, 240.0),
            monitor: Some("built-in".into()),
        });
        let encoded = source.layout_json().expect("layout json");
        let only_primary = BTreeMap::from([("built-in".to_string(), primary)]);
        let mut restored = controller();
        let update = restored
            .restore_layout_json_clamped(&encoded, &only_primary, primary)
            .expect("unplug restore");
        assert!(update.changed);
        let secondary = restored
            .layout()
            .floating
            .iter()
            .find(|dock| dock.root.contains(&DockId::from("sources")))
            .expect("sources window");
        assert_eq!(secondary.monitor, None);
        assert_eq!(
            secondary.bounds,
            DockBounds::new(1_600.0, 80.0, 400.0, 300.0).clamped_to(primary)
        );
        let primary_window = restored
            .layout()
            .floating
            .iter()
            .find(|dock| dock.root.contains(&DockId::from("mixer")))
            .expect("mixer window");
        assert_eq!(primary_window.monitor.as_deref(), Some("built-in"));
        assert_eq!(
            primary_window.bounds,
            DockBounds::new(80.0, 60.0, 360.0, 240.0)
        );

        let corrected = restored.layout_json().expect("corrected json");
        let mut again = controller();
        let second = again
            .restore_layout_json_clamped(&corrected, &only_primary, primary)
            .expect("second unplug restore");
        assert!(!second.changed);
    }

    #[test]
    fn main_surface_resize_does_not_dirty_layout_persist() {
        let mut controller = controller();
        controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(40.0, 50.0, 360.0, 280.0),
            monitor: Some("built-in".into()),
        });
        let encoded = controller.layout_json().expect("before main resize");
        let mut persist = DockLayoutPersist::with_delay(Duration::from_millis(200));
        let resized = controller.update(DockAction::SurfaceResized {
            surface: DockSurfaceId(0),
            width: 1_440.0,
            height: 900.0,
        });
        persist.note(resized.changed, Duration::from_millis(16));
        assert!(!resized.changed);
        assert!(!persist.is_dirty());
        let moved = controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(12.0, 24.0, 1_440.0, 900.0),
            monitor: Some("built-in".into()),
        });
        persist.note(moved.changed, Duration::from_millis(32));
        assert!(!moved.changed);
        assert!(!persist.is_dirty());
        assert_eq!(controller.layout_json().expect("unchanged"), encoded);
        assert_eq!(
            controller.layout().floating[0].monitor.as_deref(),
            Some("built-in")
        );
    }

    #[test]
    fn persist_throttle_coalesces_moves_and_flushes_on_exit() {
        let mut persist = DockLayoutPersist::with_delay(Duration::from_millis(200));
        let mut writes = 0;
        persist.note(true, Duration::from_millis(0));
        persist.note(true, Duration::from_millis(16));
        persist.note(true, Duration::from_millis(32));
        persist.note(true, Duration::from_millis(48));
        assert!(!persist.poll(Duration::from_millis(48)));
        assert_eq!(persist.next_wakeup(), Some(Duration::from_millis(248)));
        if persist.poll(Duration::from_millis(248)) {
            writes += 1;
        }
        persist.note(true, Duration::from_millis(250));
        persist.note(false, Duration::from_millis(251));
        if persist.flush() {
            writes += 1;
        }
        assert!(!persist.flush());
        assert_eq!(writes, 2);
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
        let now = Duration::ZERO;
        controller.update_mutation_at(
            DockMutation::SurfaceGeometry {
                surface: DockSurfaceId(0),
                bounds: DockBounds::new(40.0, 60.0, 1_000.0, 700.0),
                monitor: None,
            },
            now,
        );

        controller.update_mutation_at(
            DockMutation::ResizeStart {
                surface: DockSurfaceId(0),
                path: vec![1],
            },
            now,
        );
        controller.update_mutation_at(
            DockMutation::ResizeMove(LogicalPoint::new(600.0, 300.0)),
            now,
        );
        controller.update_mutation_at(
            DockMutation::ResizeMove(LogicalPoint::new(600.0, 1_300.0)),
            now,
        );
        let (_, maximum) =
            split_at_path(&controller.layout().main, &[1]).expect("nested split maximum");
        assert!(maximum < 1.0);

        let local_extent = 700.0 - DIVIDER_HIT_SIZE;
        controller.update_mutation_at(
            DockMutation::ResizeMove(LogicalPoint::new(600.0, 300.0 + local_extent * 0.1)),
            now,
        );
        let (_, reentered) =
            split_at_path(&controller.layout().main, &[1]).expect("nested split reentered");
        assert!((reentered - 0.85).abs() < 0.000_1);

        controller.update_mutation_at(DockMutation::ResizeEnd, now);
        controller.update_mutation_at(
            DockMutation::ResizeStart {
                surface: DockSurfaceId(0),
                path: Vec::new(),
            },
            now,
        );
        controller.update_mutation_at(
            DockMutation::ResizeMove(LogicalPoint::new(300.0, 200.0)),
            now,
        );
        controller.update_mutation_at(
            DockMutation::ResizeMove(LogicalPoint::new(-1_000.0, 200.0)),
            now,
        );
        controller.update_mutation_at(
            DockMutation::ResizeMove(LogicalPoint::new(399.2, 200.0)),
            now,
        );
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
        let now = Instant::now();
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(0.0, 0.0, 1_000.0, 760.0),
            monitor: None,
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
                position: LogicalPoint::new(100.0, 80.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(300.0, 120.0),
            },
            now + Duration::from_millis(1),
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
            preview_ready_at + Duration::from_millis(1),
        );
        assert_eq!(
            update.effects,
            vec![DockHostEffect::CloseFloating(floating.surface)]
        );
        assert!(controller.layout().floating.is_empty());
        assert!(controller.is_visible(&DockId::from("sources")));
    }

    #[test]
    fn floating_source_hits_main_beneath_the_moving_source_window() {
        let mut controller = controller();
        let opened = controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(100.0, 100.0, 360.0, 280.0),
            monitor: None,
        });
        let DockHostEffect::OpenFloating(floating) = &opened.effects[0] else {
            panic!("source floating window")
        };
        let source = floating.surface;
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(0.0, 0.0, 1_000.0, 760.0),
            monitor: None,
        });
        controller.update(DockAction::SurfaceLayout {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(0.0, 60.0, 1_000.0, 700.0),
        });

        let now = Instant::now();
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
                position: LogicalPoint::new(100.0, 100.0),
            },
            now,
        );
        let moved = controller.update_at(
            DockAction::DragMove {
                surface: source,
                position: LogicalPoint::new(200.0, 120.0),
            },
            now + Duration::from_millis(1),
        );
        assert!(moved.effects.iter().any(|effect| {
            matches!(effect, DockHostEffect::MoveFloating { surface: moved, .. } if *moved == source)
        }));
        controller.update_at(
            DockAction::DragMove {
                surface: source,
                position: LogicalPoint::new(100.0, 100.0),
            },
            now + Duration::from_millis(2),
        );

        let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        assert!(
            controller
                .drop_target()
                .is_some_and(|target| target.surface == DockSurfaceId(0))
        );
        let update = controller.update_at(
            DockAction::DragEnd { surface: source },
            preview_ready_at + Duration::from_millis(1),
        );
        assert!(update.changed);
        assert_eq!(update.effects, vec![DockHostEffect::CloseFloating(source)]);
        assert!(controller.layout().floating.is_empty());
        assert!(!controller.is_dragging());
    }

    #[test]
    fn floating_source_hits_another_floating_window_beneath_itself() {
        let (mut controller, source, target) = floating_pair_controller();
        controller.update(DockAction::SurfaceGeometry {
            surface: source,
            bounds: DockBounds::new(100.0, 100.0, 360.0, 280.0),
            monitor: None,
        });
        controller.update(DockAction::SurfaceGeometry {
            surface: target,
            bounds: DockBounds::new(500.0, 100.0, 360.0, 280.0),
            monitor: None,
        });

        let now = Instant::now();
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
                position: LogicalPoint::new(100.0, 100.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: source,
                position: LogicalPoint::new(200.0, 120.0),
            },
            now + Duration::from_millis(1),
        );
        controller.update_at(
            DockAction::DragMove {
                surface: target,
                position: LogicalPoint::new(180.0, 140.0),
            },
            now + Duration::from_millis(2),
        );
        controller.update_at(
            DockAction::DragMove {
                surface: source,
                position: LogicalPoint::new(100.0, 100.0),
            },
            now + Duration::from_millis(3),
        );

        let preview_ready_at = after_drag_dwell(now + DRAG_TEST_TICK);
        controller.update_at(DockAction::Hover(false), preview_ready_at);
        assert_eq!(
            controller.drop_target(),
            Some(&DockDropTarget {
                surface: target,
                id: "mixer".into(),
                zone: DockDropZone::Tab,
            })
        );
        let update = controller.update_at(
            DockAction::DragEnd { surface: source },
            preview_ready_at + Duration::from_millis(1),
        );
        assert!(update.changed);
        assert_eq!(update.effects, vec![DockHostEffect::CloseFloating(source)]);
        assert_eq!(controller.layout().floating.len(), 1);
        assert_eq!(controller.layout().floating[0].surface, target);
        assert!(!controller.is_dragging());
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
        let now = Instant::now();
        controller.update(DockAction::SurfaceGeometry {
            surface: DockSurfaceId(0),
            bounds: DockBounds::new(0.0, 0.0, 1_000.0, 760.0),
            monitor: None,
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
                position: LogicalPoint::new(100.0, 80.0),
            },
            now,
        );
        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(300.0, 40.0),
            },
            now + Duration::from_millis(1),
        );
        assert_eq!(controller.drop_target(), None);

        controller.update_at(
            DockAction::DragMove {
                surface: DockSurfaceId(0),
                position: LogicalPoint::new(300.0, 180.0),
            },
            now + Duration::from_millis(2),
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
        let WindowCommand::Open { id, settings } = &hosted.window_commands[0] else {
            panic!("hosted open command")
        };
        assert_eq!(*id, WindowId::from(surface));
        assert_eq!(settings.initial_position, Some((40.0, 50.0)));
        assert_eq!(settings.initial_size, (360.0, 280.0));
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
            [WindowCommand::SetBounds { id, position, size }]
                if *id == WindowId::from(surface)
                    && *position == (80.0, 90.0)
                    && *size == (360.0, 280.0)
        ));
        let restored = controller.open_hosted_windows("NanaUI Dock");
        assert_eq!(restored.window_commands.len(), 1);
        let WindowCommand::Open { settings, .. } = &restored.window_commands[0] else {
            panic!("restored hosted open command")
        };
        assert_eq!(settings.title, "NanaUI Dock");

        let geometry = WindowGeometry {
            physical_position: Some((120, 160)),
            physical_size: (800, 600),
            logical_position: Some((60.0, 80.0)),
            logical_size: (400.0, 300.0),
            scale_factor: 2.0,
            maximized: false,
        };
        let update = controller.update_hosted_window(WindowEvent::Moved {
            id: WindowId::from(surface),
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

        let close = controller.update_hosted_window(WindowEvent::CloseRequested {
            id: WindowId::from(surface),
        });
        assert_eq!(close.effects, vec![DockHostEffect::CloseFloating(surface)]);
    }

    #[cfg(feature = "hosted")]
    #[test]
    fn restored_hosted_windows_open_inside_primary_work_area() {
        let mut controller = controller();
        controller.update(DockAction::Float {
            id: "sources".into(),
            bounds: DockBounds::new(4_000.0, -200.0, 500.0, 400.0),
            monitor: Some("unplugged".into()),
        });
        let (opened, persist) = controller.open_restored_hosted_windows(
            "NanaUI Dock",
            &BTreeMap::new(),
            DockBounds::new(0.0, 0.0, 1280.0, 800.0),
        );
        assert!(persist);
        let WindowCommand::Open { settings, .. } = &opened.window_commands[0] else {
            panic!("clamped hosted open");
        };
        assert_eq!(settings.initial_position, Some((780.0, 0.0)));
        assert_eq!(settings.initial_size, (500.0, 400.0));
        assert_eq!(
            controller.layout().floating[0].bounds,
            DockBounds::new(780.0, 0.0, 500.0, 400.0)
        );
    }
}
