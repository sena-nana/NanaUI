//! The focused editor's caret blink: a paint overlay on the animation clock.
//!
//! The caret's position, the IME anchor and scrolling never see the blink;
//! only extraction leaves the caret out in the hidden half. The clock is
//! asked for a wake only while a caret blinks, so a window with nothing to
//! type into sleeps.

use std::time::Duration;

use crate::{AnimationFrame, AppContext, StableNodeId};

/// How long the caret stays shown, then hidden.
pub(crate) const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Default)]
pub(super) struct CaretBlink {
    /// When the current half ends. `None` until the clock next advances,
    /// which starts a shown half: a reset does not know the time.
    next_toggle: Option<Duration>,
    hidden: bool,
    /// A reset showed a hidden caret outside a clock advance; the next one
    /// reports it so the host draws a frame.
    revealed: bool,
    /// 富文本沿用输入时间启动每个编辑器的相位，独立于宿主布局刷新。
    rich: Vec<RichCaretBlink>,
}

#[derive(Debug)]
struct RichCaretBlink {
    node: StableNodeId,
    revision: u64,
    next: Duration,
    hidden: bool,
}

impl AppContext {
    /// The carets that blink: each source's editor, while its window has
    /// focus. A source that never heard about window focus (a headless one)
    /// counts as focused.
    fn blinking_carets(&self) -> Vec<StableNodeId> {
        let mut carets = Vec::new();
        for state in self.input.sources.values() {
            if state.disconnected || (state.focus_ever && !state.focused) {
                continue;
            }
            if let Some(node) = state.effects.caret_node()
                && self.world.rich_editor_marks(node).is_none()
                && self.world.is_mounted(node)
                && !carets.contains(&node)
            {
                carets.push(node);
            }
        }
        carets
    }

    /// Show the caret and start its blink over at the next clock advance.
    pub(super) fn reset_caret_blink(&mut self) {
        let blink = &mut self.input.caret_blink;
        blink.next_toggle = None;
        if std::mem::take(&mut blink.hidden) {
            blink.revealed = true;
        }
        self.sync_rich_caret_blinks();
    }

    /// 当前窗口中的富文本编辑器，按文档焦点和编辑状态读取，避免依赖旧输入效果快照。
    fn active_rich_carets(&self) -> Vec<(StableNodeId, u64)> {
        let mut carets = Vec::new();
        for state in self.input.sources.values() {
            if state.disconnected || (state.focus_ever && !state.focused) {
                continue;
            }
            let Some(editor) = self.focused_rich_editor(state.document) else {
                continue;
            };
            if self.world.is_mounted(editor.stable_id())
                && let Ok(Some(revision)) = self.read(editor, |editor| {
                    (!editor.read_only).then(|| editor.caret_revision())
                })
                && !carets.iter().any(|(node, _)| *node == editor.stable_id())
            {
                carets.push((editor.stable_id(), revision));
            }
        }
        carets
    }

    /// 与上游绘制覆盖层共享隐藏列表；富文本窗口失活时保留选区并隐藏光标。
    fn publish_caret_visibility(&mut self) {
        let mut hidden = if self.input.caret_blink.hidden {
            self.blinking_carets()
        } else {
            Vec::new()
        };
        hidden.extend(
            self.input
                .caret_blink
                .rich
                .iter()
                .filter(|caret| caret.hidden)
                .map(|caret| caret.node),
        );
        let active = self.active_rich_carets();
        for source in self.input.sources.values() {
            if let Some(editor) = self.focused_rich_editor(source.document)
                && !active.iter().any(|(node, _)| *node == editor.stable_id())
                && !hidden.contains(&editor.stable_id())
            {
                hidden.push(editor.stable_id());
            }
        }
        self.world.set_hidden_carets(&hidden);
    }

    /// 组件提交和焦点变化都只同步光标状态，保持文本、布局与输入法锚点稳定。
    pub(crate) fn sync_rich_caret_blinks(&mut self) {
        // update_component 提交投影时实体临时借出，待组件归还后的生命周期再读取。
        if self.input.sources.values().any(|source| {
            self.world.focused(source.document).is_some_and(|node| {
                self.world.rich_editor_marks(node).is_some() && !self.views.contains_key(&node)
            })
        }) {
            return;
        }
        let active = self.active_rich_carets();
        let now = self.component_lifecycle.now;
        let carets = &mut self.input.caret_blink.rich;
        carets.retain(|caret| active.iter().any(|(node, _)| *node == caret.node));
        for (node, revision) in active {
            if let Some(caret) = carets.iter_mut().find(|caret| caret.node == node) {
                if caret.revision != revision {
                    caret.revision = revision;
                    caret.next = now + CARET_BLINK_INTERVAL;
                    caret.hidden = false;
                }
            } else {
                carets.push(RichCaretBlink {
                    node,
                    revision,
                    next: now + CARET_BLINK_INTERVAL,
                    hidden: false,
                });
            }
        }
        self.publish_caret_visibility();
    }

    /// When the blink next changes, on the animation clock; `None` while no
    /// caret blinks.
    pub(crate) fn caret_blink_deadline(&self) -> Option<Duration> {
        let plain = (!self.blinking_carets().is_empty())
            .then_some(self.input.caret_blink.next_toggle.unwrap_or(Duration::ZERO));
        let active = self.active_rich_carets();
        plain
            .into_iter()
            .chain(
                self.input
                    .caret_blink
                    .rich
                    .iter()
                    .filter(|caret| active.iter().any(|(node, _)| *node == caret.node))
                    .map(|caret| caret.next),
            )
            .min()
    }

    /// Advance the blink to `now`, adding the editors whose caret changed
    /// to `frame` so the host draws them.
    pub(crate) fn advance_caret_blink(&mut self, now: Duration, frame: &mut AnimationFrame) {
        self.sync_rich_caret_blinks();
        for caret in &mut self.input.caret_blink.rich {
            if now >= caret.next {
                let elapsed = now.saturating_sub(caret.next).as_millis();
                let intervals = elapsed / CARET_BLINK_INTERVAL.as_millis() + 1;
                if intervals % 2 == 1 {
                    caret.hidden = !caret.hidden;
                    frame.component_updates.push(caret.node);
                }
                let remainder = elapsed % CARET_BLINK_INTERVAL.as_millis();
                caret.next = now + Duration::from_millis(500 - remainder as u64);
            }
        }
        let carets = self.blinking_carets();
        if carets.is_empty() {
            self.input.caret_blink.next_toggle = None;
            self.input.caret_blink.hidden = false;
            self.publish_caret_visibility();
            return;
        }
        let blink = &mut self.input.caret_blink;
        let mut changed = std::mem::take(&mut blink.revealed);
        match blink.next_toggle {
            None => blink.next_toggle = Some(now + CARET_BLINK_INTERVAL),
            Some(at) if now >= at => {
                blink.hidden = !blink.hidden;
                // A late wake (a stalled loop) starts the next half now
                // rather than toggling again to catch up.
                blink.next_toggle = Some(now + CARET_BLINK_INTERVAL);
                changed = true;
            }
            Some(_) => {}
        }
        self.publish_caret_visibility();
        if changed {
            for caret in carets {
                if !frame.component_updates.contains(&caret) {
                    frame.component_updates.push(caret);
                }
            }
        }
    }
}
