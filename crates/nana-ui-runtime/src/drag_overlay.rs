//! Runtime drag-and-drop visual overlay, cursor-follower, drop-zone hysteresis,
//! and cross-window drag handoff helpers.
//!
//! All geometry and timing helpers here are pure and domain-agnostic: callers
//! supply viewport bounds, zone identifiers, and theme-derived styling tokens
//! without hardcoding colors or durations inside the runtime.

use std::time::Duration;

use nana_ui_core::{
    DataTransfer, DragSessionId, DropEffect, DropEffects, DropKind, LayoutStyle, LengthSpec,
    PaintTransform, PositionSpec,
};

use crate::{AppContext, DocumentId, FrameworkError, LayoutBox, MutationQueue, StableNodeId};

/// Preferred placement of a [`CursorFollower`] relative to the pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorFollowerPlacement {
    /// Bottom-right of the pointer by `(dx, dy)`, flipping to the opposite side
    /// of the pointer whenever the overlay would overflow the viewport.
    BottomRightFlip { dx: f32, dy: f32 },
    /// Horizontally centered above the pointer by `gap`, flipping below when
    /// there is not enough room above.
    AboveCenterFlip { gap: f32 },
    /// Fixed offset from the pointer, clamped inside the viewport safe margin.
    Offset { dx: f32, dy: f32 },
    /// Centered directly on the pointer, clamped inside the viewport safe margin.
    Centered,
}

impl Default for CursorFollowerPlacement {
    fn default() -> Self {
        Self::Centered
    }
}

/// Pure viewport-clamped cursor-following overlay positioner.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CursorFollower {
    pub placement: CursorFollowerPlacement,
    /// Minimum inset from each viewport edge in logical px.
    pub viewport_inset: f32,
}

impl CursorFollower {
    pub fn new(placement: CursorFollowerPlacement) -> Self {
        Self {
            placement,
            ..Self::default()
        }
    }

    pub const fn viewport_inset(mut self, inset: f32) -> Self {
        self.viewport_inset = if inset.is_finite() && inset > 0.0 {
            inset
        } else {
            0.0
        };
        self
    }

    /// Resolve top-left `(x, y)` coordinates in viewport logical pixels for an
    /// overlay of `overlay_size = (width, height)` following `pointer = (px, py)`
    /// inside `viewport = (vw, vh)`.
    pub fn resolve(
        &self,
        pointer: (f32, f32),
        overlay_size: (f32, f32),
        viewport: (f32, f32),
    ) -> (f32, f32) {
        let (px, py) = pointer;
        let (ow, oh) = (overlay_size.0.max(0.0), overlay_size.1.max(0.0));
        let (vw, vh) = (viewport.0.max(0.0), viewport.1.max(0.0));
        let inset = self.viewport_inset;
        let max_x = (vw - inset - ow).max(inset);
        let max_y = (vh - inset - oh).max(inset);

        let (raw_x, raw_y) = match self.placement {
            CursorFollowerPlacement::BottomRightFlip { dx, dy } => {
                let mut x = px + dx;
                if x + ow > vw - inset {
                    let flipped = px - dx - ow;
                    if flipped >= inset {
                        x = flipped;
                    }
                }
                let mut y = py + dy;
                if y + oh > vh - inset {
                    let flipped = py - dy - oh;
                    if flipped >= inset {
                        y = flipped;
                    }
                }
                (x, y)
            }
            CursorFollowerPlacement::AboveCenterFlip { gap } => {
                let x = px - ow * 0.5;
                let above = py - gap - oh;
                let y = if above >= inset { above } else { py + gap };
                (x, y)
            }
            CursorFollowerPlacement::Offset { dx, dy } => (px + dx, py + dy),
            CursorFollowerPlacement::Centered => (px - ow * 0.5, py - oh * 0.5),
        };

        (raw_x.clamp(inset, max_x), raw_y.clamp(inset, max_y))
    }

    /// Apply a compositor-friendly translation transform placing `node` at the
    /// resolved cursor-follower coordinates without re-laying out the document.
    pub fn apply_transform(
        &self,
        cx: &mut AppContext,
        node: StableNodeId,
        pointer: (f32, f32),
        overlay_size: (f32, f32),
        viewport: (f32, f32),
    ) -> Result<(f32, f32), FrameworkError> {
        let (x, y) = self.resolve(pointer, overlay_size, viewport);
        let mut queue = MutationQueue::new();
        queue.patch_transform(
            node,
            Some(PaintTransform {
                e: x,
                f: y,
                ..PaintTransform::default()
            }),
        );
        cx.commit_mutations(queue)?;
        Ok((x, y))
    }

    /// Build a base [`LayoutStyle`] for a fixed, pointer-transparent overlay
    /// container positioned via [`Self::apply_transform`].
    pub fn overlay_layout_style(z_index: i32) -> LayoutStyle {
        LayoutStyle {
            position: PositionSpec::Fixed,
            offset_left: Some(LengthSpec::Px(0.0)),
            offset_top: Some(LengthSpec::Px(0.0)),
            pointer_events: Some(nana_ui_core::PointerEventsSpec::None),
            z_index: Some(z_index),
            ..LayoutStyle::default()
        }
    }
}

/// Visual state of the source item while a drag is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DragSourcePresentation {
    /// Source remains in place normally.
    #[default]
    Normal,
    /// Source is lifted (dimmed/placeholder in place while ghost follows cursor).
    Lifted,
    /// Source slot collapses out of layout while lifted.
    Collapsed,
}

/// Floating drag preview + badge + source lift/collapse coordinator.
#[derive(Debug, Clone, PartialEq)]
pub struct DragOverlay {
    pub follower: CursorFollower,
    pub active: bool,
    pub source_node: Option<StableNodeId>,
    pub source_presentation: DragSourcePresentation,
    /// Optional count or status badge shown on the floating preview (e.g. item count).
    pub badge_count: Option<usize>,
    pub badge_label: Option<String>,
    pub pointer: (f32, f32),
    pub overlay_size: (f32, f32),
    pub resolved_origin: (f32, f32),
}

impl Default for DragOverlay {
    fn default() -> Self {
        Self {
            follower: CursorFollower::default(),
            active: false,
            source_node: None,
            source_presentation: DragSourcePresentation::Normal,
            badge_count: None,
            badge_label: None,
            pointer: (0.0, 0.0),
            overlay_size: (0.0, 0.0),
            resolved_origin: (0.0, 0.0),
        }
    }
}

impl DragOverlay {
    pub fn new(follower: CursorFollower) -> Self {
        Self {
            follower,
            ..Self::default()
        }
    }

    /// Begin displaying a floating drag overlay lifted from `source_node`.
    pub fn begin(
        &mut self,
        source_node: Option<StableNodeId>,
        presentation: DragSourcePresentation,
        pointer: (f32, f32),
        overlay_size: (f32, f32),
        viewport: (f32, f32),
    ) -> (f32, f32) {
        self.active = true;
        self.source_node = source_node;
        self.source_presentation = presentation;
        self.pointer = pointer;
        self.overlay_size = overlay_size;
        self.resolved_origin = self.follower.resolve(pointer, overlay_size, viewport);
        self.resolved_origin
    }

    /// Update pointer position and recompute the clamped overlay origin.
    pub fn update_pointer(&mut self, pointer: (f32, f32), viewport: (f32, f32)) -> (f32, f32) {
        self.pointer = pointer;
        self.resolved_origin = self.follower.resolve(pointer, self.overlay_size, viewport);
        self.resolved_origin
    }

    /// Set a numeric badge (e.g., multi-item drag count; `<= 1` clears it).
    pub fn set_badge_count(&mut self, count: Option<usize>) {
        self.badge_count = count.filter(|&c| c > 1);
    }

    /// Set an optional status badge label (e.g. copy/move/detach hint).
    pub fn set_badge_label(&mut self, label: Option<impl Into<String>>) {
        self.badge_label = label.map(Into::into);
    }

    /// End the drag overlay and reset source presentation state.
    pub fn end(&mut self) {
        self.active = false;
        self.source_node = None;
        self.source_presentation = DragSourcePresentation::Normal;
        self.badge_count = None;
        self.badge_label = None;
    }

    /// True if `node` should be hidden/collapsed in layout because it is the
    /// lifted drag source in [`DragSourcePresentation::Collapsed`] mode.
    pub fn is_source_collapsed(&self, node: StableNodeId) -> bool {
        self.active
            && self.source_node == Some(node)
            && self.source_presentation == DragSourcePresentation::Collapsed
    }

    /// True if `node` is currently the lifted drag source.
    pub fn is_source_lifted(&self, node: StableNodeId) -> bool {
        self.active
            && self.source_node == Some(node)
            && matches!(
                self.source_presentation,
                DragSourcePresentation::Lifted | DragSourcePresentation::Collapsed
            )
    }
}

/// Tuning parameters for [`DropZoneRegistry`] hit-testing hysteresis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropZoneConfig {
    /// Extra hit slop expanded around each zone rect while already hovered
    /// (prevents edge jitter on narrow strips).
    pub exit_slop: f32,
    /// Minimum dwell time inside a new zone before switching active target.
    pub enter_dwell: Duration,
    /// Minimum dwell time outside the current zone before clearing it when
    /// pointer speed is below `fast_exit_speed`.
    pub exit_dwell: Duration,
    /// Pointer velocity (logical px / sec) above which leaving a zone drops it
    /// immediately without waiting for `exit_dwell`.
    pub fast_exit_speed: f32,
}

impl Default for DropZoneConfig {
    fn default() -> Self {
        Self {
            exit_slop: 0.0,
            enter_dwell: Duration::ZERO,
            exit_dwell: Duration::ZERO,
            fast_exit_speed: f32::INFINITY,
        }
    }
}

/// One registered drop zone in viewport coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct DropZone<T> {
    pub id: T,
    pub bounds: LayoutBox,
    /// Higher priority wins when two drop zones overlap.
    pub priority: i32,
}

/// Optimistic hold after a drop commits while async/remote reconciliation
/// finishes (prevents flicker back to the source position).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDrop<T> {
    pub zone: T,
    pub committed_at: Duration,
    pub expires_at: Duration,
}

impl<T> PendingDrop<T> {
    pub fn is_expired(&self, now: Duration) -> bool {
        now >= self.expires_at
    }
}

/// Output of [`DropZoneRegistry::update_pointer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropZoneDecision<T> {
    pub active: Option<T>,
    pub changed: bool,
}

/// Generic drop-zone hit-test registry with dwell / exit-velocity hysteresis
/// and optimistic [`PendingDrop`] hold tracking.
#[derive(Debug, Clone)]
pub struct DropZoneRegistry<T> {
    config: DropZoneConfig,
    zones: Vec<DropZone<T>>,
    active: Option<T>,
    candidate_enter: Option<(T, Duration)>,
    exit_started_at: Option<Duration>,
    last_sample: Option<((f32, f32), Duration)>,
    pending_drop: Option<PendingDrop<T>>,
}

impl<T> Default for DropZoneRegistry<T> {
    fn default() -> Self {
        Self::new(DropZoneConfig::default())
    }
}

impl<T> DropZoneRegistry<T> {
    pub fn new(config: DropZoneConfig) -> Self {
        Self {
            config,
            zones: Vec::new(),
            active: None,
            candidate_enter: None,
            exit_started_at: None,
            last_sample: None,
            pending_drop: None,
        }
    }

    pub fn config(&self) -> DropZoneConfig {
        self.config
    }

    pub fn set_config(&mut self, config: DropZoneConfig) {
        self.config = config;
    }

    pub fn clear_zones(&mut self) {
        self.zones.clear();
    }

    pub fn register(&mut self, id: T, bounds: LayoutBox, priority: i32) {
        self.zones.push(DropZone {
            id,
            bounds,
            priority,
        });
    }

    pub fn active(&self) -> Option<&T> {
        self.active.as_ref()
    }

    pub fn pending_drop(&self) -> Option<&PendingDrop<T>> {
        self.pending_drop.as_ref()
    }

    /// Record an optimistic drop hold lasting `hold_duration`.
    pub fn hold_pending_drop(&mut self, zone: T, now: Duration, hold_duration: Duration) {
        self.pending_drop = Some(PendingDrop {
            zone,
            committed_at: now,
            expires_at: now.saturating_add(hold_duration),
        });
    }

    /// Clear any active optimistic pending-drop hold (e.g. once state commits).
    pub fn clear_pending_drop(&mut self) -> Option<PendingDrop<T>> {
        self.pending_drop.take()
    }

    /// Expire the pending drop if `now >= expires_at`. Returns `true` if expired.
    pub fn expire_pending_drop(&mut self, now: Duration) -> bool {
        if self
            .pending_drop
            .as_ref()
            .is_some_and(|pending| pending.is_expired(now))
        {
            self.pending_drop = None;
            true
        } else {
            false
        }
    }

    /// Reset transient hover/dwell tracking (preserves registered zones and
    /// any optimistic pending drop).
    pub fn reset_hover(&mut self) {
        self.active = None;
        self.candidate_enter = None;
        self.exit_started_at = None;
        self.last_sample = None;
    }

    /// Next monotonic timestamp when a pending dwell transition will mature.
    pub fn next_deadline(&self) -> Option<Duration> {
        let enter = self
            .candidate_enter
            .as_ref()
            .map(|(_, since)| since.saturating_add(self.config.enter_dwell));
        let exit = self
            .exit_started_at
            .map(|since| since.saturating_add(self.config.exit_dwell));
        match (enter, exit) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

impl<T: Clone + PartialEq> DropZoneRegistry<T> {
    /// Update pointer position at monotonic timestamp `now` and evaluate active
    /// drop zone with dwell and exit-velocity hysteresis.
    pub fn update_pointer(&mut self, pointer: (f32, f32), now: Duration) -> DropZoneDecision<T> {
        let speed = self
            .last_sample
            .map(|((lx, ly), last_t)| {
                let dt = now.saturating_sub(last_t).as_secs_f32();
                if dt > 1e-4 {
                    (pointer.0 - lx).hypot(pointer.1 - ly) / dt
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);
        self.last_sample = Some((pointer, now));

        let hit = self.hit_test_with_slop(pointer);
        let previous = self.active.clone();

        match hit {
            Some(zone_id) => {
                self.exit_started_at = None;
                if self.active.as_ref() == Some(&zone_id) {
                    self.candidate_enter = None;
                } else if self.config.enter_dwell.is_zero() {
                    self.active = Some(zone_id);
                    self.candidate_enter = None;
                } else {
                    match &self.candidate_enter {
                        Some((cand, since)) if cand == &zone_id => {
                            if now.saturating_sub(*since) >= self.config.enter_dwell {
                                self.active = Some(zone_id);
                                self.candidate_enter = None;
                            }
                        }
                        _ => {
                            self.candidate_enter = Some((zone_id, now));
                        }
                    }
                }
            }
            None => {
                self.candidate_enter = None;
                if self.active.is_some() {
                    let fast_exit = speed >= self.config.fast_exit_speed;
                    if fast_exit || self.config.exit_dwell.is_zero() {
                        self.active = None;
                        self.exit_started_at = None;
                    } else if let Some(since) = self.exit_started_at {
                        if now.saturating_sub(since) >= self.config.exit_dwell {
                            self.active = None;
                            self.exit_started_at = None;
                        }
                    } else {
                        self.exit_started_at = Some(now);
                    }
                }
            }
        }

        let changed = self.active != previous;
        DropZoneDecision {
            active: self.active.clone(),
            changed,
        }
    }

    fn hit_test_with_slop(&self, (x, y): (f32, f32)) -> Option<T> {
        let mut best: Option<&DropZone<T>> = None;
        for zone in &self.zones {
            let slop = if self.active.as_ref() == Some(&zone.id) {
                self.config.exit_slop.max(0.0)
            } else {
                0.0
            };
            let b = zone.bounds;
            if x >= b.x - slop
                && x <= b.x + b.width + slop
                && y >= b.y - slop
                && y <= b.y + b.height + slop
                && best.is_none_or(|prev| zone.priority >= prev.priority)
            {
                best = Some(zone);
            }
        }
        best.map(|z| z.id.clone())
    }
}

/// Serializable/transferable descriptor for handing an active internal drag
/// across host windows or documents.
#[derive(Debug, Clone)]
pub struct CrossWindowDragHandoff {
    pub session: DragSessionId,
    pub source_document: DocumentId,
    pub source_node: StableNodeId,
    pub kind: DropKind,
    pub transfer: DataTransfer,
    pub allowed_effects: DropEffects,
    pub origin: (f32, f32),
    pub screen_position: (f32, f32),
    pub target_document: Option<DocumentId>,
    pub target_node: Option<StableNodeId>,
    pub negotiated_effect: Option<DropEffect>,
}

impl CrossWindowDragHandoff {
    pub fn is_alive(&self) -> bool {
        self.transfer.lifetime().is_alive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn cursor_follower_flips_on_viewport_edges_and_clamps() {
        let follower =
            CursorFollower::new(CursorFollowerPlacement::BottomRightFlip { dx: 12.0, dy: 12.0 })
                .viewport_inset(8.0);

        // Normal room: bottom-right of pointer.
        assert_eq!(
            follower.resolve((100.0, 100.0), (80.0, 30.0), (400.0, 300.0)),
            (112.0, 112.0)
        );
        // Near right & bottom edge: flips to left & above pointer.
        assert_eq!(
            follower.resolve((360.0, 280.0), (80.0, 30.0), (400.0, 300.0)),
            (268.0, 238.0)
        );

        let above = CursorFollower::new(CursorFollowerPlacement::AboveCenterFlip { gap: 10.0 })
            .viewport_inset(6.0);
        // Near top edge: flips below pointer.
        assert_eq!(
            above.resolve((200.0, 15.0), (100.0, 24.0), (400.0, 300.0)),
            (150.0, 25.0)
        );
    }

    #[test]
    fn drag_overlay_tracks_lifted_collapsed_source_and_badges() {
        let node = StableNodeId::new(42).unwrap();
        let mut overlay = DragOverlay::new(CursorFollower::new(CursorFollowerPlacement::Centered));
        let origin = overlay.begin(
            Some(node),
            DragSourcePresentation::Collapsed,
            (200.0, 150.0),
            (60.0, 20.0),
            (800.0, 600.0),
        );
        assert_eq!(origin, (170.0, 140.0));
        assert!(overlay.is_source_lifted(node));
        assert!(overlay.is_source_collapsed(node));

        overlay.set_badge_count(Some(1));
        assert_eq!(overlay.badge_count, None);
        overlay.set_badge_count(Some(3));
        assert_eq!(overlay.badge_count, Some(3));
        overlay.set_badge_label(Some("Move"));
        assert_eq!(overlay.badge_label.as_deref(), Some("Move"));

        overlay.end();
        assert!(!overlay.active);
        assert!(!overlay.is_source_lifted(node));
        assert!(!overlay.is_source_collapsed(node));
    }

    #[test]
    fn drop_zone_registry_respects_dwell_slop_velocity_and_pending_drop() {
        let mut registry = DropZoneRegistry::new(DropZoneConfig {
            exit_slop: 10.0,
            enter_dwell: ms(80),
            exit_dwell: ms(120),
            fast_exit_speed: 500.0,
        });
        registry.register(
            "trash",
            LayoutBox {
                x: 100.0,
                y: 100.0,
                width: 80.0,
                height: 40.0,
            },
            1,
        );

        // Enter starts dwell timer, not yet active.
        let d0 = registry.update_pointer((120.0, 110.0), ms(0));
        assert_eq!(d0.active, None);
        assert_eq!(registry.next_deadline(), Some(ms(80)));

        // After enter_dwell elapses, zone activates.
        let d1 = registry.update_pointer((122.0, 110.0), ms(85));
        assert_eq!(d1.active, Some("trash"));
        assert!(d1.changed);

        // Moving 5px outside stays inside exit_slop (10px), so stays active.
        let d2 = registry.update_pointer((185.0, 110.0), ms(120));
        assert_eq!(d2.active, Some("trash"));
        assert!(!d2.changed);

        // Slow exit beyond exit_slop waits for exit_dwell.
        let d3 = registry.update_pointer((195.0, 110.0), ms(220));
        assert_eq!(d3.active, Some("trash"));
        let d4 = registry.update_pointer((196.0, 110.0), ms(345));
        assert_eq!(d4.active, None);
        assert!(d4.changed);

        // Fast exit drops immediately.
        registry.update_pointer((120.0, 110.0), ms(400));
        assert_eq!(
            registry.update_pointer((120.0, 110.0), ms(490)).active,
            Some("trash")
        );
        let fast = registry.update_pointer((400.0, 110.0), ms(500));
        assert_eq!(fast.active, None);
        assert!(fast.changed);

        // Optimistic pending drop hold expires on schedule.
        registry.hold_pending_drop("trash", ms(500), ms(200));
        assert!(!registry.expire_pending_drop(ms(650)));
        assert!(registry.pending_drop().is_some());
        assert!(registry.expire_pending_drop(ms(700)));
        assert!(registry.pending_drop().is_none());
    }
}
