//! The idle hide of media transport bars that let the runtime drive it
//! ([`crate::MediaTransportBar::auto_hide`]): routed input reveals or
//! conceals them, and the animation clock hides them when idle.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::AppContext;
use crate::overlay_visibility::OverlayStep;
use crate::{DocumentId, Entity, FrameworkError, MediaTransportBar, StableNodeId};

/// What a routed event means to an auto-hidden bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum OverlayActivity {
    /// Nothing the policy reacts to at once.
    None,
    /// The pointer moved, pressed or released here, over `target` when the
    /// event landed on a node.
    Pointer {
        x: f32,
        y: f32,
        target: Option<StableNodeId>,
    },
    Key,
    /// The pointer left the window.
    Leave,
}

/// A bar whose idle hide the runtime drives.
#[derive(Debug, Default)]
pub(crate) struct AutoOverlay {
    /// The latest activity routed input saw while the bar was up and counting
    /// down to its idle hide. That activity only moves the deadline later, so
    /// the policy takes it when the deadline comes rather than on every
    /// pointer move.
    activity: Option<Instant>,
    /// `playing && !disabled` the policy last ran with; `None` before its
    /// first run.
    active: Option<bool>,
}

/// The policy keeps `Instant`s; the runtime clock is a `Duration` from the
/// host's epoch. Any fixed origin keeps their differences, which is all the
/// policy compares.
fn origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

fn instant(now: Duration) -> Instant {
    origin() + now
}

impl AppContext {
    /// Start or stop driving `bar` as `snapshot` (its current value) asks,
    /// and run its policy when what the policy reads from the bar changed
    /// since it last ran: playback started or stopped, the bar was enabled or
    /// disabled, or one of its menus opened or closed.
    pub(crate) fn sync_auto_overlay(
        &mut self,
        bar: Entity<MediaTransportBar>,
        snapshot: &MediaTransportBar,
    ) -> Result<(), FrameworkError> {
        let overlays = &mut self.component_lifecycle.auto_overlays;
        if !snapshot.auto_hide {
            overlays.remove(&bar.stable_id());
            return Ok(());
        }
        let active = snapshot.playing && !snapshot.disabled;
        let ran = overlays.entry(bar.stable_id()).or_default().active;
        if ran == Some(active) && !snapshot.menu_toggled {
            return Ok(());
        }
        self.drive_auto_overlay(bar, false, false).map(drop)
    }

    pub(crate) fn has_auto_overlays(&self) -> bool {
        !self.component_lifecycle.auto_overlays.is_empty()
    }

    /// The earliest moment a driven bar's policy wants to run again.
    pub(super) fn auto_overlay_deadline(&self) -> Option<Duration> {
        self.component_lifecycle
            .auto_overlays
            .keys()
            .filter(|bar| self.world.is_mounted(**bar))
            .filter_map(|bar| {
                self.view_entity::<MediaTransportBar>(*bar)
                    .and_then(|bar| self.read(bar, |bar| bar.visibility.wakeup()).ok())
                    .flatten()
            })
            .filter_map(|wakeup| wakeup.checked_duration_since(origin()))
            .min()
    }

    /// The driven bars mounted in `document` (any document when `None`).
    /// A bar that went away stops being driven.
    fn driven_bars(&mut self, document: Option<DocumentId>) -> Vec<Entity<MediaTransportBar>> {
        let mut bars = Vec::new();
        let mut gone = Vec::new();
        for &bar in self.component_lifecycle.auto_overlays.keys() {
            let Some(entity) = self.view_entity::<MediaTransportBar>(bar) else {
                gone.push(bar);
                continue;
            };
            if document.is_none_or(|document| self.world.document_of(bar) == Some(document))
                && self.world.is_mounted(bar)
            {
                bars.push(entity);
            }
        }
        for bar in gone {
            self.component_lifecycle.auto_overlays.remove(&bar);
        }
        bars
    }

    /// Feed a routed event to every driven bar in `document`.
    ///
    /// Activity on a bar that is up and counting down only moves its
    /// deadline, so it is noted for the policy to take when the deadline
    /// comes. Any other bar runs its policy now: one that is hidden or
    /// concealed may reveal, and one that is paused or held may have just
    /// been released.
    pub(crate) fn route_auto_overlays(&mut self, document: DocumentId, activity: OverlayActivity) {
        let now = instant(self.component_lifecycle.now);
        for bar in self.driven_bars(Some(document)) {
            let reveal = match activity {
                OverlayActivity::Pointer { x, y, target } => {
                    self.over_stage(document, bar.stable_id(), x, y, target)
                }
                OverlayActivity::Key => true,
                OverlayActivity::None | OverlayActivity::Leave => false,
            };
            let leave = activity == OverlayActivity::Leave;
            if !leave
                && self
                    .read(bar, |bar| bar.visibility.counting_down())
                    .unwrap_or(false)
            {
                if reveal
                    && let Some(driven) = self
                        .component_lifecycle
                        .auto_overlays
                        .get_mut(&bar.stable_id())
                {
                    driven.activity = Some(now);
                }
                continue;
            }
            let _ = self.drive_auto_overlay(bar, reveal, leave);
        }
    }

    /// Run the policy of every driven bar whose wakeup has come; answers
    /// the bars whose visibility flipped.
    pub(crate) fn drive_due_auto_overlays(&mut self) -> Vec<StableNodeId> {
        let now = instant(self.component_lifecycle.now);
        let mut flipped = Vec::new();
        for bar in self.driven_bars(None) {
            let due = self
                .read(bar, |bar| {
                    bar.visibility.wakeup().is_some_and(|wakeup| wakeup <= now)
                })
                .unwrap_or(false);
            if due && self.drive_auto_overlay(bar, false, false).unwrap_or(false) {
                flipped.push(bar.stable_id());
            }
        }
        flipped
    }

    /// Run `bar`'s policy at the runtime's current time, with the activity
    /// noted for it, a reveal now or the pointer leaving the window; answers
    /// whether its visibility flipped.
    fn drive_auto_overlay(
        &mut self,
        bar: Entity<MediaTransportBar>,
        reveal: bool,
        conceal: bool,
    ) -> Result<bool, FrameworkError> {
        let now = instant(self.component_lifecycle.now);
        let active = self.read(bar, |bar| bar.playing && !bar.disabled)?;
        let noted = self
            .component_lifecycle
            .auto_overlays
            .get_mut(&bar.stable_id())
            .and_then(|driven| {
                driven.active = Some(active);
                driven.activity.take()
            });
        let (flipped, _) = self.step_overlay(
            bar,
            OverlayStep {
                now,
                active,
                activity: if reveal { Some(now) } else { noted },
                conceal,
                // A control a click left focused would otherwise keep the
                // bar up for good.
                focus_visible_only: true,
            },
        )?;
        Ok(flipped)
    }

    /// Whether a pointer at `(x, y)` is over `bar`'s stage (its parent): the
    /// node it landed on, or the topmost one under it, is inside the stage,
    /// or nothing takes the pointer there and the point is in the stage's
    /// box.
    fn over_stage(
        &self,
        document: DocumentId,
        bar: StableNodeId,
        x: f32,
        y: f32,
        target: Option<StableNodeId>,
    ) -> bool {
        let stage = self.world.parent_id(bar).unwrap_or(bar);
        match target.or_else(|| self.world.hit_test(document, x, y)) {
            Some(hit) => self.world.is_descendant_or_self(hit, stage),
            None => self
                .world
                .layout_box(stage)
                .is_some_and(|frame| frame.contains(x, y)),
        }
    }
}
