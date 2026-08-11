//! Host-owned [`iced::widget::text_editor::Content`] store for L2 `Textarea`.
//!
//! The hosted Iced path retains `Element<'static>`, so [`EditorStore::content_static`]
//! extends the content lifetime for a single frame. Callers must drop that
//! element (HostedUi rebuild) before the next mutable [`EditorStore::perform`].

use std::collections::HashMap;

use iced::widget::text_editor::{self, Content};

use crate::bridge::WidgetId;

#[derive(Debug)]
struct EditorSlot {
    content: Content,
    /// Last text observed from the bridge marker (catch-up or external apply).
    bridged: String,
    /// Last settled authority version. External sync may rebuild Content only
    /// when the slot is clean and `text` advances past this (and past any
    /// active [`Self::lag_reject`] snapshot).
    confirmed: String,
    /// Armed only on dirty catch-up when the bridge marker was already equal to
    /// host text (premature [`EditorStore::acknowledge_bridge`]): the pre-edit
    /// authority is treated as a lagging v-model snapshot. Exact matches are
    /// ignored; any other bridge value applies and clears this guard. Cleared
    /// on normal align (props catch-up while `bridged` still lagged) and on
    /// stable re-align (`bridged == text` while clean).
    lag_reject: Option<String>,
    /// Host has local edits not yet confirmed by a matching bridge value.
    /// While dirty, [`Self::sync_text`] must not rebuild Content from lagging
    /// `props.value` (prepare_editors / pump_frame race).
    dirty: bool,
}

/// Caller-owned multi-line editor buffers keyed by semantic widget id.
#[derive(Debug, Default)]
pub struct EditorStore {
    slots: HashMap<WidgetId, EditorSlot>,
}

impl EditorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure(&mut self, id: WidgetId, text: &str) {
        self.slots.entry(id).or_insert_with(|| EditorSlot {
            content: Content::with_text(text),
            bridged: text.to_string(),
            confirmed: text.to_string(),
            lag_reject: None,
            dirty: false,
        });
    }

    /// Apply bridge value only when it is a confirmed external advance.
    ///
    /// Rules:
    /// - Same as last bridged snapshot while clean → settle lag window; no rebuild.
    /// - Equals current host text → bridge caught up; clear dirty, update markers
    ///   only (preserve Content / cursor). Arms [`EditorSlot::lag_reject`] with
    ///   the pre-edit `confirmed` **only** when `bridged` was already equal to
    ///   host (premature ack); normal props catch-up leaves the guard clear so a
    ///   later programmatic restore to that pre-edit string can apply.
    /// - Host is dirty → skip; stale v-model must not roll back typing.
    /// - Clean + `lag_reject` → ignore only that exact stale snapshot.
    /// - Clean, settled, and `text != confirmed` → apply external authority update.
    pub fn sync_text(&mut self, id: WidgetId, text: &str) {
        match self.slots.get_mut(&id) {
            Some(slot) => {
                if slot.bridged == text && !slot.dirty {
                    // Stable re-align: Vue caught up; lag window can close.
                    slot.lag_reject = None;
                    slot.confirmed = text.to_string();
                    return;
                }
                let host = slot.content.text();
                if host == text {
                    if slot.dirty {
                        let prior = std::mem::replace(&mut slot.confirmed, text.to_string());
                        // Premature ack advanced `bridged` before props caught up:
                        // keep rejecting that prior snapshot until re-align or a
                        // distinct external value. Normal align (bridged still
                        // lagged) must not arm — restore-to-prior would be blocked.
                        if slot.bridged == host {
                            slot.lag_reject = Some(prior);
                        } else {
                            slot.lag_reject = None;
                        }
                    } else {
                        slot.confirmed = text.to_string();
                    }
                    slot.bridged = text.to_string();
                    slot.dirty = false;
                    return;
                }
                if slot.dirty || host != slot.bridged {
                    return;
                }
                // Settled clean host: reject only the known lagging snapshot.
                if slot.lag_reject.as_deref() == Some(text) {
                    return;
                }
                // Only rebuild when bridge advances past the last confirmed
                // external/host authority version.
                if text == slot.confirmed {
                    return;
                }
                slot.content = Content::with_text(text);
                slot.bridged = text.to_string();
                slot.confirmed = text.to_string();
                slot.lag_reject = None;
                slot.dirty = false;
            }
            None => {
                self.slots.insert(
                    id,
                    EditorSlot {
                        content: Content::with_text(text),
                        bridged: text.to_string(),
                        confirmed: text.to_string(),
                        lag_reject: None,
                        dirty: false,
                    },
                );
            }
        }
    }

    /// Align the bridged marker after a completed round-trip.
    ///
    /// Does **not** clear [`EditorSlot::dirty`]: only a matching
    /// [`Self::sync_text`] (bridge value == host) confirms the external source.
    pub fn acknowledge_bridge(&mut self, id: WidgetId, text: &str) {
        if let Some(slot) = self.slots.get_mut(&id) {
            slot.bridged = text.to_string();
        }
    }

    pub fn perform(&mut self, id: WidgetId, action: text_editor::Action) {
        if let Some(slot) = self.slots.get_mut(&id) {
            slot.content.perform(action);
            slot.dirty = true;
        }
    }

    pub fn text(&self, id: WidgetId) -> String {
        self.slots
            .get(&id)
            .map(|slot| slot.content.text())
            .unwrap_or_default()
    }

    pub fn get(&self, id: WidgetId) -> Option<&Content> {
        self.slots.get(&id).map(|slot| &slot.content)
    }

    /// `'static` borrow for HostedUi `Element<'static>` construction.
    ///
    /// # Safety contract (caller)
    /// The returned reference must not be held across `&mut` methods on this
    /// store (`perform`, `sync_text`, `ensure` that replaces a slot).
    pub fn content_static(&self, id: WidgetId) -> Option<&'static Content> {
        self.slots.get(&id).map(|slot| {
            // SAFETY: HostedUiRenderer drops the previous Element before the
            // next program update mutates this store. Allocations remain stable
            // for the lifetime of `EditorStore`.
            unsafe { &*(&slot.content as *const Content) }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::text_editor::{Action, Edit};

    #[test]
    fn sync_text_skips_uncommitted_host_edits() {
        let mut store = EditorStore::new();
        store.ensure(1, "base");
        store.perform(1, Action::Edit(Edit::Insert('x')));
        let dirty = store.text(1);
        assert_ne!(dirty, "base");
        // Stale bridge value must not clobber the host buffer.
        store.sync_text(1, "base");
        assert_eq!(store.text(1), dirty);
    }

    #[test]
    fn sync_text_skips_stale_bridge_after_premature_ack() {
        // Bugbot race: dispatch acknowledged host text before JS v-model caught
        // up; prepare_editors then saw lagging props.value and rebuilt Content.
        let mut store = EditorStore::new();
        store.ensure(1, "base");
        store.perform(1, Action::Edit(Edit::Insert('x')));
        let dirty = store.text(1);
        store.acknowledge_bridge(1, &dirty);
        store.sync_text(1, "base");
        assert_eq!(store.text(1), dirty, "stale bridge must not roll back host");
    }

    #[test]
    fn sync_text_applies_programmatic_reset_to_preedit_after_align() {
        // Normal props catch-up aligns host↔bridge without arming lag_reject.
        // Vue「恢复默认」to the pre-edit string must rebuild Content.
        let mut store = EditorStore::new();
        store.ensure(1, "base");
        store.perform(1, Action::Edit(Edit::Insert('x')));
        let dirty = store.text(1);
        store.sync_text(1, &dirty); // align / clear dirty (bridged lagged → no lag arm)
        store.sync_text(1, "base"); // programmatic restore to pre-edit
        assert_eq!(
            store.text(1),
            "base",
            "restore to pre-edit after align must apply"
        );
    }

    #[test]
    fn sync_text_ignores_stale_vmodel_after_ack_and_catchup() {
        // ack 后旧 v-model patch: premature ack, then catch-up clears dirty,
        // then an old patch still must not roll back host text.
        let mut store = EditorStore::new();
        store.ensure(1, "base");
        store.perform(1, Action::Edit(Edit::Insert('x')));
        let dirty = store.text(1);
        store.acknowledge_bridge(1, &dirty);
        store.sync_text(1, &dirty); // clears dirty; arms lag_reject = "base"
        store.sync_text(1, "base"); // old v-model
        assert_eq!(
            store.text(1),
            dirty,
            "ack then stale v-model must not rebuild Content"
        );
    }

    #[test]
    fn sync_text_bridge_catchup_preserves_host_text() {
        let mut store = EditorStore::new();
        store.ensure(1, "base");
        store.perform(1, Action::Edit(Edit::Insert('x')));
        let dirty = store.text(1);
        // Bridge finally matches host; only the marker advances.
        store.sync_text(1, &dirty);
        assert_eq!(store.text(1), dirty);
        store.sync_text(1, &dirty);
        assert_eq!(store.text(1), dirty);
    }

    #[test]
    fn sync_text_applies_external_bridge_change() {
        let mut store = EditorStore::new();
        store.ensure(1, "base");
        store.sync_text(1, "from-js");
        assert_eq!(store.text(1), "from-js");
    }

    #[test]
    fn sync_text_applies_external_after_host_committed() {
        let mut store = EditorStore::new();
        store.ensure(1, "base");
        store.perform(1, Action::Edit(Edit::Insert('x')));
        let dirty = store.text(1);
        store.sync_text(1, &dirty); // catch up / clear dirty
        store.sync_text(1, &dirty); // stable re-align
        store.sync_text(1, "from-js");
        assert_eq!(store.text(1), "from-js");
    }

    #[test]
    fn sync_text_applies_external_reset_while_lag_window_open() {
        // After premature-ack catch-up arms lag_reject, a distinct programmatic
        // value must still apply immediately — do not require a re-align frame.
        let mut store = EditorStore::new();
        store.ensure(1, "base");
        store.perform(1, Action::Edit(Edit::Insert('x')));
        let dirty = store.text(1);
        store.acknowledge_bridge(1, &dirty);
        store.sync_text(1, &dirty); // catch up; lag_reject = "base"
        store.sync_text(1, "from-js"); // authoritative external reset
        assert_eq!(
            store.text(1),
            "from-js",
            "external reset after align must not be blocked by lag window"
        );
    }

    #[test]
    fn sync_text_applies_preedit_reset_after_ack_lag_realign() {
        // ack arms lag_reject; stable re-align clears it so restore-to-preedit
        // can apply (same string as the former lag snapshot).
        let mut store = EditorStore::new();
        store.ensure(1, "base");
        store.perform(1, Action::Edit(Edit::Insert('x')));
        let dirty = store.text(1);
        store.acknowledge_bridge(1, &dirty);
        store.sync_text(1, &dirty); // arms lag_reject
        store.sync_text(1, &dirty); // re-align clears lag_reject
        store.sync_text(1, "base");
        assert_eq!(
            store.text(1),
            "base",
            "pre-edit restore after ack lag re-align must apply"
        );
    }

    #[test]
    fn acknowledge_bridge_allows_matching_prepare() {
        let mut store = EditorStore::new();
        store.ensure(1, "");
        store.perform(1, Action::Edit(Edit::Insert('a')));
        let text = store.text(1);
        store.acknowledge_bridge(1, &text);
        store.sync_text(1, &text);
        assert_eq!(store.text(1), "a");
    }
}
