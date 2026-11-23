//! The idle hide of media transport bars that let the runtime drive it
//! ([`crate::MediaTransportBar::auto_hide`]): routed input reveals or
//! conceals them, and the animation clock hides them when idle.

use crate::framework::via;
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
    /// Refresh every bar bound to a time-sampled resource whose resource time
    /// moved since the bar last showed it; answers the bars refreshed. A bar
    /// that went away stops being followed.
    pub(crate) fn sync_media_bound_bars(&mut self) -> Vec<StableNodeId> {
        if self.component_lifecycle.media_bound_bars.is_empty() {
            return Vec::new();
        }
        let mut moved = Vec::new();
        let mut gone = Vec::new();
        for (&bar, &shown) in &self.component_lifecycle.media_bound_bars {
            let Some(entity) = self.view_entity::<MediaTransportBar>(bar) else {
                gone.push(bar);
                continue;
            };
            if !self.world.is_mounted(bar) {
                continue;
            }
            let now = self
                .read(entity, |bar| bar.source)
                .ok()
                .flatten()
                .and_then(|source| self.world.media_time(source));
            if now != shown {
                moved.push(entity);
            }
        }
        for bar in gone {
            self.component_lifecycle.media_bound_bars.remove(&bar);
        }
        moved
            .into_iter()
            .filter(|&bar| self.sync_media_transport_bar(bar).is_ok())
            .map(|bar| bar.stable_id())
            .collect()
    }

    /// Follow `bar`'s source from now on, remembering the resource time it
    /// shows (`Some`), or stop following it (`None`, an unbound bar).
    pub(crate) fn note_media_bound_bar(
        &mut self,
        bar: Entity<MediaTransportBar>,
        shown: Option<Option<f64>>,
    ) {
        let bars = &mut self.component_lifecycle.media_bound_bars;
        match shown {
            Some(shown) => {
                bars.insert(bar.stable_id(), shown);
            }
            None => {
                bars.remove(&bar.stable_id());
            }
        }
    }

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
        via!(self.drive_auto_overlay(bar, None)).map(drop)
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
                    Some(self.over_stage(document, bar.stable_id(), x, y, target))
                }
                OverlayActivity::Key => Some(true),
                OverlayActivity::Leave => Some(false),
                OverlayActivity::None => None,
            };
            if reveal != Some(false)
                && self
                    .read(bar, |bar| bar.visibility.counting_down())
                    .unwrap_or(false)
            {
                if reveal == Some(true)
                    && let Some(driven) = self
                        .component_lifecycle
                        .auto_overlays
                        .get_mut(&bar.stable_id())
                {
                    driven.activity = Some(now);
                }
                continue;
            }
            let _ = via!(self.drive_auto_overlay(bar, reveal));
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
            if due && via!(self.drive_auto_overlay(bar, None)).unwrap_or(false) {
                flipped.push(bar.stable_id());
            }
        }
        flipped
    }

    /// Run `bar`'s policy at the runtime's current time, with the activity
    /// noted for it, a reveal now or the pointer leaving the stage; answers
    /// whether its visibility flipped.
    pub(crate) fn drive_auto_overlay(
        &mut self,
        bar: Entity<MediaTransportBar>,
        reveal: Option<bool>,
    ) -> Result<bool, FrameworkError> {
        let now = instant(self.component_lifecycle.now);
        let (active, concealed) = self.read(bar, |bar| {
            (bar.playing && !bar.disabled, bar.visibility.concealed())
        })?;
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
                activity: if reveal == Some(true) {
                    Some(now)
                } else {
                    noted
                },
                // Menu close restarts activity in the policy; preserve its
                // existing conceal request unless new input replaces it.
                conceal: reveal.map_or(concealed, |reveal| !reveal),
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
        // Captured events keep their original target even outside the stage.
        // Compare in layout coordinates so ancestor scrolling and transforms
        // follow the same projection as hit testing.
        if !self
            .world
            .pointer_layout_position(stage, x, y)
            .zip(self.world.layout_box(stage))
            .is_some_and(|((x, y), frame)| frame.contains(x, y))
        {
            return false;
        }
        match target.or_else(|| self.world.hit_test(document, x, y)) {
            Some(hit) => self.world.is_descendant_or_self(hit, stage),
            None => true,
        }
    }
}
