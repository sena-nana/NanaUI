//! Backend-neutral tab-strip drag/reorder lease.
//!
//! The group owns no application ordering. It registers each strip's painted
//! bounds through a generation lease, maps window-local coordinates into
//! physical screen space, and resolves one pointer release to one
//! source/target/before. Application code owns tab values, persistence, and
//! whether a close or transfer is applied.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::geometry::LogicalPoint;

/// Shared geometry and active-drag state for tabs that can move between strips.
pub struct TabDragGroup<T> {
    inner: Arc<Mutex<TabDragGroupState<T>>>,
}

/// Window-local coordinate transform used by a [`TabDragGroup`].
#[derive(Debug, Clone, PartialEq)]
pub struct TabDragSurface {
    id: String,
    physical_origin: LogicalPoint,
    scale_factor: f32,
}

/// Axis-aligned rectangle in the same space as the corresponding [`TabDragSurface`]
/// input (window-local logical pixels before mapping).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabDragRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Painted strip snapshot registered with a [`TabDragGroup`] for one lease.
#[derive(Debug, Clone, PartialEq)]
pub struct TabStripPaint<T> {
    pub bounds: TabDragRect,
    pub tab_bounds: Vec<TabDragRect>,
    pub values: Vec<T>,
    pub disabled: Vec<bool>,
    pub accepts_external_drop: bool,
}

/// Insert-before indicator for a live drag over one registered strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabDropIndicator {
    pub before: Option<usize>,
    pub source: Option<usize>,
}

/// Short-lived registration of one painted strip. Dropping an older lease does
/// not remove a newer registration for the same strip id.
pub struct TabDragLease<T> {
    pub group: TabDragGroup<T>,
    pub surface: TabDragSurface,
    pub strip_id: String,
    pub generation: u64,
}

struct TabDragGroupState<T> {
    next_generation: u64,
    strips: BTreeMap<String, TabStripRegistration<T>>,
    active: Option<ActiveGroupDrag>,
    completed_source: Option<(String, u64)>,
}

struct TabStripRegistration<T> {
    generation: u64,
    surface_id: String,
    bounds: TabDragRect,
    tab_bounds: Vec<TabDragRect>,
    values: Vec<T>,
    disabled: Vec<bool>,
    accepts_external_drop: bool,
}

#[derive(Debug, Clone)]
struct ActiveGroupDrag {
    source_surface: String,
    source_strip: String,
    source_generation: u64,
    source_index: usize,
    position: LogicalPoint,
    moved: bool,
}

impl TabDragSurface {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            physical_origin: LogicalPoint::default(),
            scale_factor: 1.0,
        }
    }

    /// Sets the window origin in physical screen pixels and its logical scale.
    pub fn with_physical_geometry(mut self, x: i32, y: i32, scale_factor: f64) -> Self {
        self.physical_origin = LogicalPoint::new(x as f32, y as f32);
        self.scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor as f32
        } else {
            1.0
        };
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn map_point(&self, point: LogicalPoint) -> LogicalPoint {
        LogicalPoint::new(
            self.physical_origin.x + point.x * self.scale_factor,
            self.physical_origin.y + point.y * self.scale_factor,
        )
    }

    pub fn map_rect(&self, rectangle: TabDragRect) -> TabDragRect {
        let origin = self.map_point(LogicalPoint::new(rectangle.x, rectangle.y));
        TabDragRect::new(
            origin.x,
            origin.y,
            rectangle.width * self.scale_factor,
            rectangle.height * self.scale_factor,
        )
    }
}

impl TabDragRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(self, point: LogicalPoint) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && point.x >= self.x
            && point.y >= self.y
            && point.x < self.x + self.width
            && point.y < self.y + self.height
    }

    pub fn center_x(self) -> f32 {
        self.x + self.width / 2.0
    }
}

impl<T> Clone for TabDragGroup<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Default for TabDragGroup<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Default for TabDragGroupState<T> {
    fn default() -> Self {
        Self {
            next_generation: 0,
            strips: BTreeMap::new(),
            active: None,
            completed_source: None,
        }
    }
}

impl<T> TabDragGroup<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TabDragGroupState::default())),
        }
    }

    pub fn lease(&self, surface: TabDragSurface, strip_id: impl Into<String>) -> TabDragLease<T> {
        let mut state = lock(&self.inner);
        state.next_generation = state.next_generation.saturating_add(1);
        let generation = state.next_generation;
        TabDragLease {
            group: self.clone(),
            surface,
            strip_id: strip_id.into(),
            generation,
        }
    }

    pub fn register(
        &self,
        surface: &TabDragSurface,
        strip_id: &str,
        generation: u64,
        paint: TabStripPaint<T>,
    ) {
        lock(&self.inner).strips.insert(
            strip_id.to_owned(),
            TabStripRegistration {
                generation,
                surface_id: surface.id.clone(),
                bounds: surface.map_rect(paint.bounds),
                tab_bounds: paint
                    .tab_bounds
                    .into_iter()
                    .map(|bounds| surface.map_rect(bounds))
                    .collect(),
                values: paint.values,
                disabled: paint.disabled,
                accepts_external_drop: paint.accepts_external_drop,
            },
        );
    }

    pub fn sync_active(
        &self,
        surface: &TabDragSurface,
        source_strip: &str,
        source_generation: u64,
        source_index: usize,
        position: LogicalPoint,
        moved: bool,
    ) {
        let mut state = lock(&self.inner);
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.source_strip != source_strip)
        {
            return;
        }
        state.active = Some(ActiveGroupDrag {
            source_surface: surface.id.clone(),
            source_strip: source_strip.to_owned(),
            source_generation,
            source_index,
            position: surface.map_point(position),
            moved,
        });
        state.completed_source = None;
    }

    pub fn clear_active(&self, source_strip: &str, source_generation: u64) {
        let mut state = lock(&self.inner);
        if state.active.as_ref().is_some_and(|active| {
            active.source_strip == source_strip && active.source_generation == source_generation
        }) {
            state.active = None;
        }
    }

    pub fn relay_pointer(&self, surface: &TabDragSurface, position: LogicalPoint) -> bool {
        let mut state = lock(&self.inner);
        let Some(active) = state.active.as_mut() else {
            return false;
        };
        if active.source_surface == surface.id {
            return false;
        }
        active.position = surface.map_point(position);
        active.moved = true;
        true
    }

    pub fn take_completed(&self, source_strip: &str, source_generation: u64) -> bool {
        let mut state = lock(&self.inner);
        if state.completed_source.as_ref().is_some_and(|completed| {
            completed.0 == source_strip && completed.1 == source_generation
        }) {
            state.completed_source = None;
            true
        } else {
            false
        }
    }
}

impl<T: Clone> TabDragGroup<T> {
    pub fn cross_drop(
        &self,
        surface: &TabDragSurface,
        source_strip: &str,
        position: LogicalPoint,
    ) -> Option<(String, Option<T>)> {
        let position = surface.map_point(position);
        let state = lock(&self.inner);
        let (target_id, target) = state.strips.iter().find(|(strip_id, strip)| {
            strip_id.as_str() != source_strip
                && strip.accepts_external_drop
                && strip.bounds.contains(position)
        })?;
        let before = drop_before_index(&target.tab_bounds, &target.disabled, None, Some(position))
            .and_then(|index| target.values.get(index).cloned());
        Some((target_id.clone(), before))
    }

    pub fn indicator_for(&self, strip_id: &str) -> Option<TabDropIndicator> {
        let state = lock(&self.inner);
        let active = state.active.as_ref().filter(|active| active.moved)?;
        let strip = state.strips.get(strip_id)?;
        if active.source_strip != strip_id && !strip.accepts_external_drop {
            return None;
        }
        if !strip.bounds.contains(active.position) {
            return None;
        }
        let excluded = (active.source_strip == strip_id).then_some(active.source_index);
        Some(TabDropIndicator {
            before: drop_before_index(
                &strip.tab_bounds,
                &strip.disabled,
                excluded,
                Some(active.position),
            ),
            source: excluded,
        })
    }

    pub fn is_active_over(
        &self,
        surface: &TabDragSurface,
        strip_id: &str,
        position: LogicalPoint,
    ) -> bool {
        let position = surface.map_point(position);
        let state = lock(&self.inner);
        state.active.as_ref().is_some_and(|active| {
            active.moved
                && state.strips.get(strip_id).is_some_and(|strip| {
                    (active.source_strip == strip_id || strip.accepts_external_drop)
                        && strip.bounds.contains(position)
                })
        })
    }

    pub fn finish_relay(
        &self,
        surface: &TabDragSurface,
        target_strip: &str,
        position: LogicalPoint,
    ) -> Option<(String, T, String, Option<T>)> {
        let position = surface.map_point(position);
        let mut state = lock(&self.inner);
        let active = state.active.clone()?;
        if active.source_surface == surface.id {
            return None;
        }
        let source = state.strips.get(&active.source_strip)?;
        if source.generation != active.source_generation {
            return None;
        }
        let value = source.values.get(active.source_index)?.clone();
        let target = state.strips.get(target_strip)?;
        if target.surface_id != surface.id
            || !target.accepts_external_drop
            || !target.bounds.contains(position)
        {
            return None;
        }
        let before = drop_before_index(&target.tab_bounds, &target.disabled, None, Some(position))
            .and_then(|index| target.values.get(index).cloned());
        state.active = None;
        state.completed_source = Some((active.source_strip.clone(), active.source_generation));
        Some((active.source_strip, value, target_strip.to_owned(), before))
    }
}

impl<T> Drop for TabDragLease<T> {
    fn drop(&mut self) {
        let mut state = lock(&self.group.inner);
        if state
            .strips
            .get(&self.strip_id)
            .is_some_and(|strip| strip.generation == self.generation)
        {
            state.strips.remove(&self.strip_id);
        }
        if state.active.as_ref().is_some_and(|active| {
            active.source_strip == self.strip_id && active.source_generation == self.generation
        }) {
            state.active = None;
        }
    }
}

fn lock<T>(inner: &Mutex<TabDragGroupState<T>>) -> std::sync::MutexGuard<'_, TabDragGroupState<T>> {
    inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn tab_at(bounds: &[TabDragRect], disabled: &[bool], position: LogicalPoint) -> Option<usize> {
    bounds
        .iter()
        .enumerate()
        .find(|(index, bounds)| {
            !disabled.get(*index).copied().unwrap_or(true) && bounds.contains(position)
        })
        .map(|(index, _)| index)
}

pub fn drop_before_index(
    bounds: &[TabDragRect],
    disabled: &[bool],
    excluded: Option<usize>,
    position: Option<LogicalPoint>,
) -> Option<usize> {
    let position = position?;
    bounds
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            Some(*index) != excluded && !disabled.get(*index).copied().unwrap_or(false)
        })
        .find(|(_, bounds)| position.x < bounds.center_x())
        .map(|(index, _)| index)
}

pub fn reorder_changes_position(length: usize, source: usize, before: Option<usize>) -> bool {
    if source >= length || before == Some(source) {
        return false;
    }
    let mut reordered = (0..length)
        .filter(|index| *index != source)
        .collect::<Vec<_>>();
    let insert_at = before
        .and_then(|before| reordered.iter().position(|index| *index == before))
        .unwrap_or(reordered.len());
    reordered.insert(insert_at, source);
    reordered.into_iter().ne(0..length)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Vec<TabDragRect> {
        (0..3)
            .map(|index| TabDragRect::new(index as f32 * 80.0, 0.0, 76.0, 28.0))
            .collect()
    }

    #[test]
    fn reorder_contract_filters_noop_insertions() {
        assert!(!reorder_changes_position(3, 0, Some(1)));
        assert!(!reorder_changes_position(3, 1, Some(2)));
        assert!(!reorder_changes_position(3, 2, None));
        assert!(reorder_changes_position(3, 2, Some(0)));
        assert!(reorder_changes_position(3, 0, None));
    }

    #[test]
    fn disabled_tabs_are_skipped_as_drop_targets() {
        assert_eq!(
            drop_before_index(
                &bounds(),
                &[true, false, false],
                Some(2),
                Some(LogicalPoint::new(1.0, 14.0)),
            ),
            Some(1)
        );
    }

    #[test]
    fn drag_group_resolves_another_strip_and_its_before_value() {
        let group = TabDragGroup::new();
        let surface = TabDragSurface::new("default");
        let source = group.lease(surface.clone(), "left");
        let target = group.lease(surface.clone(), "right");
        let source_bounds = bounds();
        let target_bounds = bounds()
            .into_iter()
            .map(|bounds| TabDragRect::new(bounds.x + 300.0, 0.0, bounds.width, bounds.height))
            .collect::<Vec<_>>();
        group.register(
            &surface,
            &source.strip_id,
            source.generation,
            TabStripPaint {
                bounds: TabDragRect::new(0.0, 0.0, 236.0, 28.0),
                tab_bounds: source_bounds,
                values: vec!["overview", "a", "b"],
                disabled: vec![true, false, false],
                accepts_external_drop: true,
            },
        );
        group.register(
            &surface,
            &target.strip_id,
            target.generation,
            TabStripPaint {
                bounds: TabDragRect::new(300.0, 0.0, 236.0, 28.0),
                tab_bounds: target_bounds,
                values: vec!["overview", "c", "d"],
                disabled: vec![true, false, false],
                accepts_external_drop: true,
            },
        );
        let position = LogicalPoint::new(301.0, 14.0);

        assert_eq!(
            group.cross_drop(&surface, "left", position),
            Some(("right".to_owned(), Some("c")))
        );
        group.sync_active(&surface, "left", source.generation, 2, position, true);
        assert_eq!(
            group
                .indicator_for("right")
                .map(|indicator| indicator.before),
            Some(Some(1))
        );
        assert!(group.is_active_over(&surface, "right", position));
    }

    #[test]
    fn newer_strip_lease_survives_an_older_view_drop() {
        let group = TabDragGroup::<u8>::new();
        let surface = TabDragSurface::new("default");
        let older = group.lease(surface.clone(), "pane");
        group.register(
            &surface,
            &older.strip_id,
            older.generation,
            TabStripPaint {
                bounds: TabDragRect::new(0.0, 0.0, 80.0, 28.0),
                tab_bounds: vec![bounds()[0]],
                values: vec![1],
                disabled: vec![false],
                accepts_external_drop: true,
            },
        );
        let newer = group.lease(surface.clone(), "pane");
        group.register(
            &surface,
            &newer.strip_id,
            newer.generation,
            TabStripPaint {
                bounds: TabDragRect::new(0.0, 0.0, 80.0, 28.0),
                tab_bounds: vec![bounds()[0]],
                values: vec![2],
                disabled: vec![false],
                accepts_external_drop: true,
            },
        );

        drop(older);
        let state = lock(&group.inner);
        assert_eq!(state.strips["pane"].generation, newer.generation);
    }

    #[test]
    fn drag_group_relays_between_scaled_window_surfaces_once() {
        let group = TabDragGroup::new();
        let source_surface =
            TabDragSurface::new("source-window").with_physical_geometry(100, 100, 2.0);
        let target_surface =
            TabDragSurface::new("target-window").with_physical_geometry(500, 120, 1.5);
        let source = group.lease(source_surface.clone(), "source-pane");
        let target = group.lease(target_surface.clone(), "target-pane");
        group.register(
            &source_surface,
            &source.strip_id,
            source.generation,
            TabStripPaint {
                bounds: TabDragRect::new(0.0, 0.0, 236.0, 28.0),
                tab_bounds: bounds(),
                values: vec![0, 1, 2],
                disabled: vec![true, false, false],
                accepts_external_drop: false,
            },
        );
        group.register(
            &target_surface,
            &target.strip_id,
            target.generation,
            TabStripPaint {
                bounds: TabDragRect::new(0.0, 0.0, 236.0, 28.0),
                tab_bounds: bounds(),
                values: vec![0, 3, 4],
                disabled: vec![true, false, false],
                accepts_external_drop: true,
            },
        );
        group.sync_active(
            &source_surface,
            &source.strip_id,
            source.generation,
            2,
            LogicalPoint::new(198.0, 14.0),
            true,
        );

        assert!(group.relay_pointer(&target_surface, LogicalPoint::new(1.0, 14.0)));
        assert_eq!(
            group.finish_relay(
                &target_surface,
                &target.strip_id,
                LogicalPoint::new(1.0, 14.0)
            ),
            Some((
                "source-pane".to_owned(),
                2,
                "target-pane".to_owned(),
                Some(3),
            ))
        );
        assert!(group.take_completed(&source.strip_id, source.generation));
        assert!(!group.take_completed(&source.strip_id, source.generation));
    }
}
