//! Host scroll contract for JS `scrollIntoView` / `scrollTop` / `scrollLeft`.
//!
//! Algorithm (honest subset, not full CSSOM):
//! 1. Walk ancestors via the document tree.
//! 2. Treat nodes with `overflow(-x|-y): auto|scroll` as scroll containers
//!    ([`LayoutStyle::scrolls_y`] / overflow-x).
//! 3. Use iced/measure [`LayoutBox`] geometry to compute the delta that brings
//!    the target into the ancestor scrollport (`block` / `inline` align).
//! 4. Persist offsets in [`ScrollOffsetStore`], translate descendant layout
//!    boxes so `getBoundingClientRect` / `layoutBox` match the scrolled frame,
//!    and enqueue iced `scrollable` ops for hosts that drain
//!    [`drain_pending_scroll_tasks`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use nana_ui_core::LayoutStyle;

use crate::bridge::{MessageBridge, WidgetId};
use crate::tree::{LayoutBox, LayoutBoxStore, NanaTreeDocument, NodeHandle, get_layout_box_from};

/// Absolute scroll offset for one scroll container (CSS px).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollOffset {
    pub x: f32,
    pub y: f32,
}

/// Process-wide scroll offsets keyed by node / widget id.
#[derive(Debug, Default)]
pub struct ScrollOffsetStore {
    offsets: Mutex<HashMap<u64, ScrollOffset>>,
    pending: Mutex<Vec<PendingScroll>>,
}

/// One iced `scrollable` scroll-to request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingScroll {
    pub widget_id: WidgetId,
    pub offset: ScrollOffset,
}

impl ScrollOffsetStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: WidgetId) -> ScrollOffset {
        self.offsets
            .lock()
            .ok()
            .and_then(|g| g.get(&id).copied())
            .unwrap_or_default()
    }

    pub fn set(&self, id: WidgetId, offset: ScrollOffset) {
        if let Ok(mut guard) = self.offsets.lock() {
            if offset.x == 0.0 && offset.y == 0.0 {
                guard.remove(&id);
            } else {
                guard.insert(id, offset);
            }
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.offsets.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.pending.lock() {
            guard.clear();
        }
    }

    fn enqueue_pending(&self, widget_id: WidgetId, offset: ScrollOffset) {
        if let Ok(mut guard) = self.pending.lock() {
            if let Some(slot) = guard.iter_mut().find(|p| p.widget_id == widget_id) {
                slot.offset = offset;
            } else {
                guard.push(PendingScroll { widget_id, offset });
            }
        }
    }

    /// Take pending iced scroll ops (for demo / VueHost Task drain).
    pub fn take_pending(&self) -> Vec<PendingScroll> {
        self.pending
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default()
    }
}

/// Shared store used by host ops and iced-view scrollable ids.
pub fn shared_scroll_offset_store() -> Arc<ScrollOffsetStore> {
    static STORE: OnceLock<Arc<ScrollOffsetStore>> = OnceLock::new();
    Arc::clone(STORE.get_or_init(|| Arc::new(ScrollOffsetStore::new())))
}

/// iced widget id string for a scroll container node.
pub fn scrollable_widget_id(widget_id: WidgetId) -> String {
    format!("nana-scroll-{widget_id}")
}

/// Align keyword for `scrollIntoView` block / inline axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollAlign {
    #[default]
    Start,
    Center,
    End,
    Nearest,
}

impl ScrollAlign {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "center" => Self::Center,
            "end" => Self::End,
            "nearest" => Self::Nearest,
            _ => Self::Start,
        }
    }
}

/// Options subset for Element.scrollIntoView.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrollIntoViewOptions {
    pub block: ScrollAlign,
    pub inline: ScrollAlign,
}

impl ScrollIntoViewOptions {
    pub fn from_host_value(value: Option<&nana_js_engine::HostValue>) -> Self {
        let Some(value) = value else {
            return Self::default();
        };
        match value {
            nana_js_engine::HostValue::Bool(true) | nana_js_engine::HostValue::String(_) => {
                Self::default()
            }
            nana_js_engine::HostValue::Object(map) => {
                let block = map
                    .get("block")
                    .and_then(|v| v.as_str())
                    .map(ScrollAlign::parse)
                    .unwrap_or_default();
                let inline = map
                    .get("inline")
                    .and_then(|v| v.as_str())
                    .map(ScrollAlign::parse)
                    .unwrap_or(ScrollAlign::Nearest);
                Self { block, inline }
            }
            _ => Self::default(),
        }
    }
}

/// Result of applying scrollIntoView (for tests / diagnostics).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScrollIntoViewResult {
    pub scrolled: Vec<(WidgetId, ScrollOffset)>,
}

/// Whether the bridge node is a scroll container on either axis.
pub fn is_scroll_container(bridge: &MessageBridge, id: WidgetId) -> bool {
    bridge
        .get(id)
        .map(|w| scrolls_axis(&w.props.layout))
        .unwrap_or(false)
}

fn scrolls_axis(layout: &LayoutStyle) -> bool {
    layout.scrolls_y() || layout.overflow_x.scrolls()
}

/// Apply `scrollIntoView` for `target` using layout boxes + scrollable ancestors.
pub fn scroll_into_view(
    doc: &mut NanaTreeDocument,
    bridge: &MessageBridge,
    layout_store: &LayoutBoxStore,
    scroll_store: &ScrollOffsetStore,
    target: NodeHandle,
    opts: ScrollIntoViewOptions,
) -> ScrollIntoViewResult {
    let mut result = ScrollIntoViewResult::default();
    let mut ancestor = doc.parent_node(target);
    while let Some(anc) = ancestor {
        let anc_id = anc.0;
        if let Some(widget) = bridge.get(anc_id) {
            let layout = &widget.props.layout;
            if scrolls_axis(layout) {
                if let Some(applied) = scroll_ancestor_to_target(
                    doc,
                    layout_store,
                    scroll_store,
                    anc,
                    target,
                    opts,
                    layout.scrolls_y(),
                    layout.overflow_x.scrolls(),
                ) {
                    result.scrolled.push((anc_id, applied));
                }
            }
        }
        ancestor = doc.parent_node(anc);
    }
    result
}

fn scroll_ancestor_to_target(
    doc: &mut NanaTreeDocument,
    layout_store: &LayoutBoxStore,
    scroll_store: &ScrollOffsetStore,
    ancestor: NodeHandle,
    target: NodeHandle,
    opts: ScrollIntoViewOptions,
    scrolls_y: bool,
    scrolls_x: bool,
) -> Option<ScrollOffset> {
    let a = get_layout_box_from(layout_store, doc, ancestor)?;
    let t = get_layout_box_from(layout_store, doc, target)?;
    let current = scroll_store.get(ancestor.0);

    let dy = if scrolls_y {
        axis_delta(opts.block, t.y, t.height, a.y, a.height)
    } else {
        0.0
    };
    let dx = if scrolls_x {
        axis_delta(opts.inline, t.x, t.width, a.x, a.width)
    } else {
        0.0
    };

    if dx.abs() < 0.5 && dy.abs() < 0.5 {
        return None;
    }

    let next = ScrollOffset {
        x: (current.x + dx).max(0.0),
        y: (current.y + dy).max(0.0),
    };
    let applied_dx = next.x - current.x;
    let applied_dy = next.y - current.y;
    if applied_dx.abs() < 0.5 && applied_dy.abs() < 0.5 {
        return None;
    }

    scroll_store.set(ancestor.0, next);
    scroll_store.enqueue_pending(ancestor.0, next);
    translate_descendants(doc, layout_store, ancestor, -applied_dx, -applied_dy);
    Some(next)
}

fn axis_delta(
    align: ScrollAlign,
    target_pos: f32,
    target_size: f32,
    port_pos: f32,
    port_size: f32,
) -> f32 {
    let target_end = target_pos + target_size;
    let port_end = port_pos + port_size;
    match align {
        ScrollAlign::Start => target_pos - port_pos,
        ScrollAlign::End => target_end - port_end,
        ScrollAlign::Center => (target_pos + target_size * 0.5) - (port_pos + port_size * 0.5),
        ScrollAlign::Nearest => {
            if target_pos >= port_pos && target_end <= port_end {
                0.0
            } else if target_pos < port_pos {
                target_pos - port_pos
            } else {
                target_end - port_end
            }
        }
    }
}

fn translate_descendants(
    doc: &mut NanaTreeDocument,
    layout_store: &LayoutBoxStore,
    ancestor: NodeHandle,
    dx: f32,
    dy: f32,
) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    let mut stack = doc.children_of(ancestor);
    let mut updated: Vec<(NodeHandle, LayoutBox)> = Vec::new();
    while let Some(child) = stack.pop() {
        if let Some(mut box_) = get_layout_box_from(layout_store, doc, child) {
            box_.x += dx;
            box_.y += dy;
            layout_store.record(child, box_.x, box_.y, box_.width, box_.height);
            updated.push((child, box_));
        }
        stack.extend(doc.children_of(child));
    }
    if !updated.is_empty() {
        doc.apply_layout_boxes(&updated);
    }
}

/// Re-apply stored scroll offsets onto layout boxes after iced paint writeback.
///
/// HostedProgram does not drain iced `scrollable` Tasks, so paint boxes are
/// unscrolled; this restores JS-visible geometry from [`ScrollOffsetStore`].
pub fn reapply_scroll_translations(
    doc: &mut NanaTreeDocument,
    bridge: &MessageBridge,
    layout_store: &LayoutBoxStore,
    scroll_store: &ScrollOffsetStore,
) {
    let offsets: Vec<(WidgetId, ScrollOffset)> = {
        let Ok(guard) = scroll_store.offsets.lock() else {
            return;
        };
        guard
            .iter()
            .filter(|(_, o)| o.x.abs() >= 0.5 || o.y.abs() >= 0.5)
            .map(|(&id, o)| (id, *o))
            .collect()
    };
    for (id, off) in offsets {
        if !is_scroll_container(bridge, id) {
            continue;
        }
        translate_descendants(doc, layout_store, NodeHandle(id), -off.x, -off.y);
    }
}

/// Set scroll offset and translate descendant layout boxes by the delta.
pub fn set_scroll_offset(
    doc: &mut NanaTreeDocument,
    layout_store: &LayoutBoxStore,
    scroll_store: &ScrollOffsetStore,
    id: WidgetId,
    next: ScrollOffset,
) -> ScrollOffset {
    let prev = scroll_store.get(id);
    let next = ScrollOffset {
        x: next.x.max(0.0),
        y: next.y.max(0.0),
    };
    let dx = next.x - prev.x;
    let dy = next.y - prev.y;
    scroll_store.set(id, next);
    if dx.abs() >= 0.5 || dy.abs() >= 0.5 {
        scroll_store.enqueue_pending(id, next);
        translate_descendants(doc, layout_store, NodeHandle(id), -dx, -dy);
    }
    next
}

/// Drain pending iced scroll Tasks (no-op when `iced-view` is off).
#[cfg(feature = "iced-view")]
pub fn drain_pending_scroll_tasks<Message: 'static + Send>() -> iced::Task<Message> {
    use iced::widget::{Id, operation, scrollable::AbsoluteOffset};

    let pending = shared_scroll_offset_store().take_pending();
    if pending.is_empty() {
        return iced::Task::none();
    }
    iced::Task::batch(pending.into_iter().map(|p| {
        operation::scroll_to(
            Id::from(scrollable_widget_id(p.widget_id)),
            AbsoluteOffset {
                x: Some(p.offset.x),
                y: Some(p.offset.y),
            },
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{MessageBridge, WidgetKind, WidgetProps};
    use nana_ui_core::{LengthSpec, OverflowSpec};

    fn seed_scroll_tree() -> (
        NanaTreeDocument,
        MessageBridge,
        LayoutBoxStore,
        NodeHandle,
        NodeHandle,
    ) {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let body = doc.mount_root();
        let scroller = doc.create_element("div");
        let target = doc.create_element("div");
        doc.insert(scroller, body, None);
        doc.insert(target, scroller, None);

        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.layout.overflow_y = OverflowSpec::Auto;
        props.layout.height = Some(LengthSpec::Px(200.0));
        bridge.register(scroller.0, WidgetKind::Column, props);
        bridge.register(target.0, WidgetKind::Text, WidgetProps::default());

        let layout_store = LayoutBoxStore::new();
        // Scroller viewport at y=0 height 200; target starts at y=400 (below fold).
        layout_store.record(scroller, 0.0, 0.0, 300.0, 200.0);
        layout_store.record(target, 0.0, 400.0, 300.0, 40.0);
        doc.apply_layout_boxes(&layout_store.snapshot());

        (doc, bridge, layout_store, scroller, target)
    }

    #[test]
    fn scroll_into_view_block_start_brings_target_into_port() {
        let (mut doc, bridge, layout_store, scroller, target) = seed_scroll_tree();
        let scroll_store = ScrollOffsetStore::new();
        let result = scroll_into_view(
            &mut doc,
            &bridge,
            &layout_store,
            &scroll_store,
            target,
            ScrollIntoViewOptions {
                block: ScrollAlign::Start,
                inline: ScrollAlign::Nearest,
            },
        );
        assert_eq!(result.scrolled.len(), 1);
        assert_eq!(result.scrolled[0].0, scroller.0);
        assert!((result.scrolled[0].1.y - 400.0).abs() < 0.5);
        let box_ = get_layout_box_from(&layout_store, &doc, target).expect("target box");
        assert!(
            (box_.y - 0.0).abs() < 0.5,
            "target should sit at scrollport top after block:start, got y={}",
            box_.y
        );
        let port = get_layout_box_from(&layout_store, &doc, scroller).expect("port");
        assert!(
            box_.y >= port.y - 0.5 && box_.y + box_.height <= port.y + port.height + 0.5,
            "target must be visible inside scrollport"
        );
    }

    #[test]
    fn scroll_into_view_nearest_noop_when_already_visible() {
        let (mut doc, bridge, layout_store, scroller, target) = seed_scroll_tree();
        layout_store.record(target, 0.0, 40.0, 300.0, 40.0);
        doc.apply_layout_boxes(&layout_store.snapshot());
        let scroll_store = ScrollOffsetStore::new();
        let result = scroll_into_view(
            &mut doc,
            &bridge,
            &layout_store,
            &scroll_store,
            target,
            ScrollIntoViewOptions {
                block: ScrollAlign::Nearest,
                inline: ScrollAlign::Nearest,
            },
        );
        assert!(result.scrolled.is_empty());
        assert_eq!(scroll_store.get(scroller.0).y, 0.0);
    }

    #[test]
    fn set_scroll_offset_updates_store_and_pending() {
        let (mut doc, _bridge, layout_store, scroller, target) = seed_scroll_tree();
        let scroll_store = ScrollOffsetStore::new();
        set_scroll_offset(
            &mut doc,
            &layout_store,
            &scroll_store,
            scroller.0,
            ScrollOffset { x: 0.0, y: 120.0 },
        );
        assert!((scroll_store.get(scroller.0).y - 120.0).abs() < 0.5);
        let box_ = get_layout_box_from(&layout_store, &doc, target).expect("target");
        assert!((box_.y - 280.0).abs() < 0.5);
        let pending = scroll_store.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].widget_id, scroller.0);
    }
}
