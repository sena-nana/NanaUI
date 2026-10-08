//! The idle hide of media transport bars that let the runtime drive it
//! ([`crate::MediaTransportBar::auto_hide`]): routed input reveals or
//! conceals them, and the animation clock hides them when idle.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::AppContext;
use crate::{DocumentId, Entity, FrameworkError, MediaTransportBar, StableNodeId};

/// What a routed event means to an auto-hidden bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum OverlayActivity {
    /// Nothing but time and state: playback started, a menu closed.
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
    /// Start or stop driving `bar`; answers whether it is driven.
    pub(crate) fn track_auto_overlay(&mut self, bar: StableNodeId, auto: bool) -> bool {
        let overlays = &mut self.component_lifecycle.auto_overlays;
        if auto {
            overlays.insert(bar);
        } else {
            overlays.remove(&bar);
        }
        auto
    }

    pub(crate) fn has_auto_overlays(&self) -> bool {
        !self.component_lifecycle.auto_overlays.is_empty()
    }

    /// The earliest moment a driven bar's policy wants to run again.
    pub(super) fn auto_overlay_deadline(&self) -> Option<Duration> {
        self.component_lifecycle
            .auto_overlays
            .iter()
            .filter(|bar| self.world.is_mounted(**bar))
            .filter_map(|bar| {
                self.view_entity::<MediaTransportBar>(*bar)
                    .and_then(|bar| self.read(bar, |bar| bar.visibility.wakeup()).ok())
                    .flatten()
            })
            .filter_map(|wakeup| wakeup.checked_duration_since(origin()))
            .min()
    }

    /// Drive every bar in `document` (every document when `None`); answers
    /// the bars whose visibility flipped. A bar that went away stops being
    /// driven.
    pub(crate) fn drive_auto_overlays(
        &mut self,
        document: Option<DocumentId>,
        activity: OverlayActivity,
    ) -> Vec<StableNodeId> {
        let bars: Vec<_> = self
            .component_lifecycle
            .auto_overlays
            .iter()
            .copied()
            .collect();
        let mut flipped = Vec::new();
        for bar in bars {
            let Some(node) = self.world.node(bar) else {
                self.component_lifecycle.auto_overlays.remove(&bar);
                continue;
            };
            if document.is_some_and(|document| node.document != document)
                || !self.world.is_mounted(bar)
            {
                continue;
            }
            let Some(entity) = self.view_entity::<MediaTransportBar>(bar) else {
                self.component_lifecycle.auto_overlays.remove(&bar);
                continue;
            };
            if self.drive_auto_overlay(entity, activity).unwrap_or(false) {
                flipped.push(bar);
            }
        }
        flipped
    }

    /// Run `bar`'s policy at the runtime's current time; answers whether its
    /// visibility flipped.
    pub(crate) fn drive_auto_overlay(
        &mut self,
        bar: Entity<MediaTransportBar>,
        activity: OverlayActivity,
    ) -> Result<bool, FrameworkError> {
        let root = bar.stable_id();
        let node = self
            .world
            .node(root)
            .ok_or(FrameworkError::MissingView(root))?;
        let (document, stage) = (node.document, node.parent.unwrap_or(root));
        let now = instant(self.component_lifecycle.now);
        let mut locks = self.overlay_locks(document, root);
        // Keyboard focus holds the bar; a control a click left focused would
        // otherwise keep it up for good.
        locks.focused = self
            .world
            .focus_visible(document)
            .is_some_and(|focused| self.world.is_descendant_or_self(focused, root));
        let reveal = match activity {
            OverlayActivity::Pointer { x, y, target } => {
                self.over_stage(document, stage, x, y, target)
            }
            OverlayActivity::Key => true,
            OverlayActivity::None | OverlayActivity::Leave => false,
        };
        let leave = activity == OverlayActivity::Leave;
        self.update_component(bar, |bar, cx| {
            let before = bar.visibility.visible();
            let active = bar.playing && !bar.disabled;
            let menu_closed = bar.menu_was_open && !locks.menu_open;
            if reveal || menu_closed {
                bar.visibility.activity(now);
            }
            bar.visibility.synchronize(now, active, locks);
            if leave {
                bar.visibility.conceal();
            }
            bar.visibility.tick(now);
            bar.menu_was_open = locks.menu_open;
            crate::overlay_visibility::report_visibility(before, bar, cx);
            before != bar.visibility.visible()
        })
    }

    /// Whether a pointer at `(x, y)` is over `stage`: the node it landed on,
    /// or the topmost one under it, is inside the stage, or nothing takes
    /// the pointer there and the point is in the stage's box.
    fn over_stage(
        &self,
        document: DocumentId,
        stage: StableNodeId,
        x: f32,
        y: f32,
        target: Option<StableNodeId>,
    ) -> bool {
        match target.or_else(|| self.world.hit_test(document, x, y)) {
            Some(hit) => self.world.is_descendant_or_self(hit, stage),
            None => self.world.layout_box(stage).is_some_and(|frame| {
                x >= frame.x
                    && x < frame.x + frame.width
                    && y >= frame.y
                    && y < frame.y + frame.height
            }),
        }
    }
}
