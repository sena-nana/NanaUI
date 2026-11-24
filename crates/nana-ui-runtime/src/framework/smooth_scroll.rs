//! Line-wheel interaction state. ScrollOffset remains the effective position;
//! the destination is transient input state, never a second layout or transform.

use super::*;

const DURATION: Duration = Duration::from_millis(180);
const INTERVAL: Duration = Duration::from_millis(8);

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct WheelScroll {
    from: ScrollOffset,
    to: ScrollOffset,
    last: ScrollOffset,
    started: Duration,
    next: Duration,
}

impl AppContext {
    pub(super) fn wheel_scroll_deadline(&self) -> Option<Duration> {
        self.component_lifecycle
            .wheel_scrolls
            .iter()
            .filter(|(id, _)| self.wheel_scroll_live(**id))
            .map(|(_, scroll)| scroll.next)
            .min()
    }

    fn wheel_scroll_live(&self, id: StableNodeId) -> bool {
        self.world.is_mounted(id)
            && self.world.is_overlay_reachable(id)
            && (self.is_scroll_view(id)
                || self.overflow_scrolls(id)
                || self.store.views.get(&id).is_some_and(|view| {
                    view.is::<TextArea>() || view.is::<crate::RichTextEditor>()
                }))
    }

    /// Wheel-only routing; the public pixel-offset APIs keep their immediate
    /// semantics. A blocking overlay bounds the ancestor walk as before.
    pub(super) fn wheel_scroll_from(
        &mut self,
        document: DocumentId,
        hit: Option<StableNodeId>,
        delta: ScrollOffset,
        lines: bool,
        overlay: bool,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        if !lines || self.reduced_motion() {
            return if overlay {
                match hit {
                    Some(hit) => self.scroll_overlay_from(document, hit, delta),
                    None => Ok(None),
                }
            } else {
                self.scroll_from_hit(hit, delta)
            };
        }
        let root = if overlay {
            let Some(root) = self
                .active_runtime_overlays(document)
                .into_iter()
                .find(|overlay| overlay.kind.blocks_pointer())
                .map(|overlay| overlay.root)
            else {
                return Ok(None);
            };
            if hit.is_none_or(|hit| !self.overlay_descendant(root, hit)) {
                return Ok(None);
            }
            Some(root)
        } else {
            None
        };
        // A wheel is input: it runs on the host's clock, not the document's
        // presentation time, which may be paused.
        let now = self.world.presentation_host_now();
        let mut current = hit;
        let mut hops = 0;
        while let Some(id) = current {
            let next = self.wheel_next(id, &mut hops);
            if self.wheel_scroll_node(id, delta, now)? {
                return Ok(Some(id));
            }
            if Some(id) == root {
                break;
            }
            current = next;
        }
        Ok(None)
    }

    fn wheel_scroll_node(
        &mut self,
        id: StableNodeId,
        delta: ScrollOffset,
        now: Duration,
    ) -> Result<bool, FrameworkError> {
        if !self.wheel_scroll_live(id) {
            return Ok(false);
        }
        let area = self
            .store
            .views
            .get(&id)
            .is_some_and(|view| view.is::<TextArea>() || view.is::<crate::RichTextEditor>());
        if area && self.world.text_scroll_metrics(id).is_none() {
            return Ok(false);
        }
        // Catch up to the event's absolute time before retargeting.
        self.sample_wheel_scroll(id, now)?;
        if !self.wheel_scroll_live(id) {
            return Ok(false);
        }
        let current = self.world.scroll_offset(id).unwrap_or_default();
        let previous = self
            .component_lifecycle
            .wheel_scrolls
            .get(&id)
            .map_or(current, |s| s.to);
        let (x, y) = if area {
            (true, true)
        } else if self.is_scroll_view(id) {
            self.read(Entity::<ScrollView>::from_stable_id(id), |scroll| {
                (
                    matches!(scroll.axes, ScrollAxes::Horizontal | ScrollAxes::Both),
                    matches!(scroll.axes, ScrollAxes::Vertical | ScrollAxes::Both),
                )
            })?
        } else {
            self.overflow_axes(id).unwrap_or((false, false))
        };
        let destination = |current: f32, target: f32, delta: f32, enabled: bool| {
            if !enabled {
                return current;
            }
            if delta == 0.0 {
                return target;
            }
            let remaining = target - current;
            let base = if remaining * delta < 0.0 {
                current
            } else {
                target
            };
            base + delta
        };
        let to = self.world.clamp_scroll_offset(
            id,
            ScrollOffset {
                x: destination(current.x, previous.x, delta.x, x),
                y: destination(current.y, previous.y, delta.y, y),
            },
        );
        // Consumption follows the destination, so another notch at the edge
        // bubbles even while this container is still approaching its edge.
        if to == previous {
            return Ok(false);
        }
        self.component_lifecycle.wheel_scrolls.insert(
            id,
            WheelScroll {
                from: current,
                to,
                last: current,
                started: now,
                next: now.saturating_add(INTERVAL),
            },
        );
        if let Some(hold) = self
            .behavior(id)
            .and_then(|b| b.pointer_ops)
            .and_then(|p| p.hold_scrollbars)
        {
            hold(self, id)?;
        }
        Ok(true)
    }

    fn sample_wheel_scroll(
        &mut self,
        id: StableNodeId,
        now: Duration,
    ) -> Result<bool, FrameworkError> {
        let Some(mut scroll) = self.component_lifecycle.wheel_scrolls.get(&id).copied() else {
            return Ok(false);
        };
        let current = self.world.scroll_offset(id).unwrap_or_default();
        if !self.wheel_scroll_live(id) || current != self.world.clamp_scroll_offset(id, scroll.last)
        {
            self.component_lifecycle.wheel_scrolls.remove(&id);
            return Ok(false);
        }
        let to = self.world.clamp_scroll_offset(id, scroll.to);
        if to != scroll.to {
            scroll.from = current;
            scroll.to = to;
            scroll.started = now;
        }
        let elapsed = now.saturating_sub(scroll.started);
        let finished = elapsed >= DURATION || current == scroll.to;
        let offset = if finished {
            scroll.to
        } else {
            let progress = self
                .world
                .node_theme(id)
                .easing(nana_ui_core::EasingRole::Standard)
                .sample(elapsed.as_secs_f32() / DURATION.as_secs_f32());
            self.world.clamp_scroll_offset(
                id,
                ScrollOffset {
                    x: scroll.from.x + (scroll.to.x - scroll.from.x) * progress,
                    y: scroll.from.y + (scroll.to.y - scroll.from.y) * progress,
                },
            )
        };
        scroll.next = now
            .saturating_add(INTERVAL)
            .min(scroll.started.saturating_add(DURATION));
        scroll.last = offset;
        self.component_lifecycle.wheel_scrolls.insert(id, scroll);
        let changed = self.apply_scroll_node_to(id, offset)?;
        // An event observer may have requested an immediate position or removed
        // the node. Never resurrect the gesture after that callback.
        if self.component_lifecycle.wheel_scrolls.get(&id) == Some(&scroll) {
            if finished {
                self.component_lifecycle.wheel_scrolls.remove(&id);
            } else if let Some(state) = self.component_lifecycle.wheel_scrolls.get_mut(&id) {
                state.last = self.world.scroll_offset(id).unwrap_or(offset);
            }
        }
        Ok(changed)
    }

    pub(super) fn advance_wheel_scrolls(&mut self, now: Duration, frame: &mut AnimationFrame) {
        let mut due = std::mem::take(&mut self.component_lifecycle.wheel_scroll_due);
        due.extend(
            self.component_lifecycle
                .wheel_scrolls
                .iter()
                .filter(|(id, scroll)| scroll.next <= now || !self.wheel_scroll_live(**id))
                .map(|(id, _)| *id),
        );
        for id in due.drain(..) {
            match self.sample_wheel_scroll(id, now) {
                Ok(true) => frame.component_updates.push(id),
                Ok(false) => {}
                Err(_) => {
                    self.component_lifecycle.wheel_scrolls.remove(&id);
                }
            }
        }
        self.component_lifecycle.wheel_scroll_due = due;
    }

    pub(super) fn finish_wheel_scrolls(&mut self) {
        let pending = std::mem::take(&mut self.component_lifecycle.wheel_scrolls);
        for (id, scroll) in pending {
            if !self.wheel_scroll_live(id) {
                continue;
            }
            let _ = self.apply_scroll_node_to(id, scroll.to);
        }
    }
}
