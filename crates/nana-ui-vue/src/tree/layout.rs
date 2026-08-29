//! Layout box store and document geometry helpers for the Vue tree document.
//!
//! Split out of `tree.rs`: JS paint-phase geometry is a per-window concern,
//! independent of the retained document core.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use super::NodeHandle;
use super::NanaTreeDocument;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomNodeKind {
    Element,
    Text,
    Comment,
    Document,
    Other,
}

/// Layout box in logical CSS px (viewport / Scene absolute coordinates).
///
/// Sources, in preference order for JS `getBoundingClientRect` / `layoutBox`:
/// 1. Scene paint writeback ([`LayoutBoxStore`])
/// 2. Style-Model [`crate::measure_layout`] applied via [`NanaTreeDocument::apply_layout_boxes`]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBox {
    pub handle: NodeHandle,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Per-window JS paint-phase geometry. Not layout authority.
///
/// Incremental Scene paint via [`Self::record`]; scroll lives in a JS overlay
/// and is not written back to Runtime `LayoutBox`.
#[derive(Debug, Default)]
pub struct LayoutBoxStore {
    boxes: Mutex<HashMap<u64, LayoutBox>>,
    views: Mutex<HashMap<u64, LayoutBox>>,
    transforms: Mutex<HashMap<u64, (LayoutBox, [f32; 6])>>,
}

impl LayoutBoxStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop JS scroll overlays; keep last Scene paint boxes.
    pub fn begin_frame(&self) {
        if let Ok(mut guard) = self.views.lock() {
            guard.clear();
        }
    }

    pub fn record(&self, handle: NodeHandle, x: f32, y: f32, width: f32, height: f32) {
        if let Ok(mut guard) = self.boxes.lock() {
            guard.insert(
                handle.0,
                LayoutBox {
                    handle,
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
        self.clear_view(handle);
        if let Ok(mut guard) = self.transforms.lock() {
            guard.remove(&handle.0);
        }
    }

    pub fn record_transformed(
        &self,
        handle: NodeHandle,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        affine: [f32; 6],
    ) {
        let source = LayoutBox {
            handle,
            x,
            y,
            width,
            height,
        };
        let transformed = transform_layout_box(source, affine);
        if let Ok(mut guard) = self.boxes.lock() {
            guard.insert(handle.0, transformed);
        }
        self.clear_view(handle);
        if let Ok(mut guard) = self.transforms.lock() {
            guard.insert(handle.0, (source, affine));
        }
    }

    pub fn remove(&self, handle: NodeHandle) {
        if let Ok(mut guard) = self.boxes.lock() {
            guard.remove(&handle.0);
        }
        self.clear_view(handle);
        if let Ok(mut guard) = self.transforms.lock() {
            guard.remove(&handle.0);
        }
    }

    pub fn retain(&self, mut live: impl FnMut(u64) -> bool) {
        if let Ok(mut guard) = self.boxes.lock() {
            guard.retain(|&id, _| live(id));
        }
        if let Ok(mut guard) = self.views.lock() {
            guard.retain(|&id, _| live(id));
        }
        if let Ok(mut guard) = self.transforms.lock() {
            guard.retain(|&id, _| live(id));
        }
    }

    fn clear_view(&self, handle: NodeHandle) {
        if let Ok(mut guard) = self.views.lock() {
            guard.remove(&handle.0);
        }
    }

    pub fn contains_point(&self, handle: NodeHandle, x: f32, y: f32) -> bool {
        if self.view_box(handle).is_some() {
            return self.get(handle).is_some_and(|box_| {
                x >= box_.x && y >= box_.y && x < box_.x + box_.width && y < box_.y + box_.height
            });
        }
        let transformed = self
            .transforms
            .lock()
            .ok()
            .and_then(|guard| guard.get(&handle.0).copied());
        let Some((source, affine)) = transformed else {
            return self.get(handle).is_some_and(|box_| {
                x >= box_.x && y >= box_.y && x < box_.x + box_.width && y < box_.y + box_.height
            });
        };
        inverse_affine_point(x, y, affine).is_some_and(|(local_x, local_y)| {
            local_x >= source.x
                && local_y >= source.y
                && local_x < source.x + source.width
                && local_y < source.y + source.height
        })
    }

    pub fn local_point(&self, handle: NodeHandle, x: f32, y: f32) -> Option<(f32, f32)> {
        if let Some(box_) = self.view_box(handle) {
            return Some((x - box_.x, y - box_.y));
        }
        let transformed = self
            .transforms
            .lock()
            .ok()
            .and_then(|guard| guard.get(&handle.0).copied());
        match transformed {
            Some((source, affine)) => {
                inverse_affine_point(x, y, affine).map(|(px, py)| (px - source.x, py - source.y))
            }
            None => self.get(handle).map(|box_| (x - box_.x, y - box_.y)),
        }
    }

    pub fn translate(&self, handle: NodeHandle, dx: f32, dy: f32) -> Option<LayoutBox> {
        let mut box_ = self.get(handle)?;
        box_.x += dx;
        box_.y += dy;
        self.overlay_view(box_);
        Some(box_)
    }

    pub(crate) fn overlay_view(&self, box_: LayoutBox) {
        if let Ok(mut views) = self.views.lock() {
            views.insert(box_.handle.0, box_);
        }
    }

    fn view_box(&self, handle: NodeHandle) -> Option<LayoutBox> {
        self.views
            .lock()
            .ok()
            .and_then(|guard| guard.get(&handle.0).copied())
    }

    pub fn get(&self, handle: NodeHandle) -> Option<LayoutBox> {
        self.view_box(handle).or_else(|| self.source_box(handle))
    }

    /// Scene paint box, ignoring JS scroll / sticky overlays.
    pub fn source_box(&self, handle: NodeHandle) -> Option<LayoutBox> {
        self.boxes
            .lock()
            .ok()
            .and_then(|g| g.get(&handle.0).copied())
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.lock().map(|g| g.is_empty()).unwrap_or(true)
    }

    pub fn len(&self) -> usize {
        self.boxes.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn snapshot(&self) -> Vec<(NodeHandle, LayoutBox)> {
        let Ok(guard) = self.boxes.lock() else {
            return Vec::new();
        };
        let mut out: Vec<(NodeHandle, LayoutBox)> =
            guard.iter().map(|(&id, b)| (NodeHandle(id), *b)).collect();
        out.sort_by_key(|(h, _)| h.0);
        out
    }
}

fn inverse_affine_point(x: f32, y: f32, [a, b, c, d, e, f]: [f32; 6]) -> Option<(f32, f32)> {
    let determinant = a * d - b * c;
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    let px = x - e;
    let py = y - f;
    Some((
        (d * px - c * py) / determinant,
        (-b * px + a * py) / determinant,
    ))
}

fn transform_layout_box(source: LayoutBox, [a, b, c, d, e, f]: [f32; 6]) -> LayoutBox {
    let corners = [
        (source.x, source.y),
        (source.x + source.width, source.y),
        (source.x, source.y + source.height),
        (source.x + source.width, source.y + source.height),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        let tx = a * x + c * y + e;
        let ty = b * x + d * y + f;
        min_x = min_x.min(tx);
        min_y = min_y.min(ty);
        max_x = max_x.max(tx);
        max_y = max_y.max(ty);
    }
    LayoutBox {
        handle: source.handle,
        x: min_x,
        y: min_y,
        width: (max_x - min_x).max(0.0),
        height: (max_y - min_y).max(0.0),
    }
}

/// Prefer `store` writeback, else the document's pre-paint measure cache.
pub fn get_layout_box_from(
    store: &LayoutBoxStore,
    doc: &NanaTreeDocument,
    handle: NodeHandle,
) -> Option<LayoutBox> {
    store.get(handle).or_else(|| doc.layout_box(handle))
}

/// JS `scrollWidth` / `scrollHeight`: Runtime metrics, else descendant union.
pub fn query_scroll_content_size(
    store: &LayoutBoxStore,
    doc: &NanaTreeDocument,
    node: NodeHandle,
    client_width: f32,
    client_height: f32,
) -> (f32, f32) {
    if let Some(metrics) = doc.scroll_metrics(node) {
        return (
            metrics.content_width.max(client_width).max(0.0),
            metrics.content_height.max(client_height).max(0.0),
        );
    }
    let Some(viewport) = get_layout_box_from(store, doc, node) else {
        return (client_width.max(0.0), client_height.max(0.0));
    };
    let (content_width, content_height) =
        union_descendant_content(doc, node, viewport, |doc, child| {
            get_layout_box_from(store, doc, child)
        });
    (
        content_width.max(client_width),
        content_height.max(client_height),
    )
}

fn union_descendant_content(
    doc: &NanaTreeDocument,
    node: NodeHandle,
    viewport: LayoutBox,
    mut box_of: impl FnMut(&NanaTreeDocument, NodeHandle) -> Option<LayoutBox>,
) -> (f32, f32) {
    let mut content_width = viewport.width;
    let mut content_height = viewport.height;
    let mut stack = doc.children_of(node);
    while let Some(child) = stack.pop() {
        if let Some(box_) = box_of(doc, child) {
            content_width = content_width.max(box_.x + box_.width - viewport.x);
            content_height = content_height.max(box_.y + box_.height - viewport.y);
        }
        stack.extend(doc.children_of(child));
    }
    (content_width.max(0.0), content_height.max(0.0))
}

/// Document layout cache (pre-paint measure or last [`NanaTreeDocument::apply_layout_boxes`]).
///
/// Live Scene writeback is on the per-window [`LayoutBoxStore`]; use [`get_layout_box_from`].
pub fn get_layout_box(doc: &NanaTreeDocument, handle: NodeHandle) -> Option<LayoutBox> {
    doc.layout_box(handle)
}

/// Compact dump used by headless probes.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxSnapshot {
    pub boxes: Vec<LayoutBox>,
    pub texts: Vec<(NodeHandle, String)>,
    pub tags: Vec<(NodeHandle, String)>,
    pub event_targets: HashSet<(u64, String)>,
    pub gpu_slots: Vec<(NodeHandle, String)>,
}
