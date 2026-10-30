//! The timer behind an in-app menu bar's submenu aim: a pointer that rests
//! inside the triangle on its way to a submenu hands the highlight to the row
//! under it once the grace period runs out. See [`crate::submenu_aim`].

use std::time::Duration;

use crate::{AppContext, AppMenuBar, StableNodeId};

impl AppContext {
    /// The clock pointer input was last stamped with.
    pub(crate) fn menu_aim_clock(&self) -> Duration {
        self.component_lifecycle.now
    }

    /// Remember when `bar`'s aim runs out, or that it has none.
    pub(crate) fn note_menu_aim(&mut self, bar: StableNodeId, until: Option<Duration>) {
        match until {
            Some(until) => {
                self.component_lifecycle.menu_aims.insert(bar, until);
            }
            None => {
                self.component_lifecycle.menu_aims.remove(&bar);
            }
        }
    }

    pub(super) fn menu_aim_deadline(&self) -> Option<Duration> {
        self.component_lifecycle
            .menu_aims
            .iter()
            .filter(|(bar, _)| self.world.is_mounted(**bar))
            .map(|(_, until)| *until)
            .min()
    }

    /// Settle the aims that ran out by `now`. Returns the bars that changed.
    pub(super) fn settle_menu_aims(&mut self, now: Duration) -> Vec<StableNodeId> {
        let due: Vec<_> = self
            .component_lifecycle
            .menu_aims
            .iter()
            .filter(|(bar, until)| **until <= now || !self.world.is_mounted(**bar))
            .map(|(bar, _)| *bar)
            .collect();
        let mut settled = Vec::with_capacity(due.len());
        for id in due {
            let bar = self
                .view_entity::<AppMenuBar>(id)
                .filter(|_| self.world.is_mounted(id));
            let Some(bar) = bar else {
                self.component_lifecycle.menu_aims.remove(&id);
                continue;
            };
            let changed = self
                .drive_menu_bar(bar, |bar| {
                    let before = bar.state.clone();
                    bar.settle_aim(now);
                    bar.state != before
                })
                .unwrap_or(false);
            if changed {
                settled.push(id);
            }
            // the bar's own state is the authority: a model swapped under an
            // aim leaves an entry that must not keep waking frames
            let until = self
                .read(bar, |bar| bar.state.aim.map(|aim| aim.aim.until()))
                .ok()
                .flatten();
            self.note_menu_aim(id, until);
        }
        settled
    }
}
