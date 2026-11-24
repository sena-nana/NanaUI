//! The keys an in-app menu bar answers to: Alt pressed and released alone
//! or F10 give it the keyboard, and while it is open or holds the keyboard
//! the arrows, Enter and Escape are its own. See [`crate::AppMenuBar`].

use nana_ui_input::InputModifiers;

use crate::{AppContext, AppMenuBar, DocumentId, FrameworkError, StableNodeId};

impl AppContext {
    /// Remember whether `bar` is `document`'s active menu bar.
    pub(crate) fn note_menu_bar(&mut self, document: DocumentId, bar: StableNodeId, active: bool) {
        if active {
            self.input.menu_bars.insert(document, bar);
        } else if self.input.menu_bars.get(&document) == Some(&bar) {
            self.input.menu_bars.remove(&document);
        }
    }

    /// A menu bar or its layer: a press there leaves the focus where it is.
    pub(super) fn is_menu_bar_node(&self, id: StableNodeId) -> bool {
        self.store.views.get(&id).is_some_and(|view| {
            view.is::<AppMenuBar>() || view.is::<crate::app_menu_bar::MenuBarScrim>()
        })
    }

    /// A press anywhere ends an Alt that was going to reach the menu bar.
    pub(crate) fn cancel_menu_bar_alt(&mut self) {
        self.input.menu_alt = false;
    }

    /// A key transition before anything else sees it. Answers whether a
    /// menu bar took it.
    pub(crate) fn route_menu_bar_key(
        &mut self,
        document: DocumentId,
        key: &str,
        pressed: bool,
        repeat: bool,
        modifiers: InputModifiers,
    ) -> Result<bool, FrameworkError> {
        if key == "Alt" {
            if pressed {
                if !repeat {
                    self.input.menu_alt = true;
                }
                return Ok(false);
            }
            if !std::mem::take(&mut self.input.menu_alt) {
                return Ok(false);
            }
            let Some(bar) = self.keyed_menu_bar(document) else {
                return Ok(false);
            };
            let alt_key = self.read(bar, |bar| bar.alt_key || bar.is_active())?;
            return if alt_key && let Some(menu_bar) = self.capabilities.menu_bar {
                (menu_bar.toggle_keyboard)(self, bar.stable_id())
            } else {
                Ok(false)
            };
        }
        if !pressed {
            return Ok(false);
        }
        self.input.menu_alt = false;
        let plain = !modifiers.alt && !modifiers.control && !modifiers.meta && !modifiers.shift;
        if key == "F10" && plain {
            return match (self.keyed_menu_bar(document), self.capabilities.menu_bar) {
                (Some(bar), Some(menu_bar)) => (menu_bar.toggle_keyboard)(self, bar.stable_id()),
                _ => Ok(false),
            };
        }
        let Some(bar) = self
            .input
            .menu_bars
            .get(&document)
            .copied()
            .and_then(|id| self.active_menu_bar(id))
        else {
            return Ok(false);
        };
        self.menu_bar_key(bar, key)
    }

    /// The active menu bar of `document`, or its first.
    fn keyed_menu_bar(&self, document: DocumentId) -> Option<crate::Entity<AppMenuBar>> {
        let candidates = self
            .store
            .views
            .iter()
            .filter(|(_, view)| view.is::<AppMenuBar>())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        self.menu_bar_for_keys(
            document,
            self.input.menu_bars.get(&document).copied(),
            candidates.into_iter(),
        )
    }
}
