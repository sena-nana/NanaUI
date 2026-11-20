//! Vertical list with thresholded reorder and optional tree-drop intents.
//!
//! Application code owns item identities, order, grouping, and persistence.
//! This type reports select, before-value reorder, and tree-drop results from
//! pointer geometry and the keyboard. Rows may stand for a flattened tree:
//! each row's depth says which rows below it it holds, and a drag carries
//! that subtree with it.

use std::sync::Arc;

#[cfg(test)]
use nana_ui_core::UI_METRICS;
use nana_ui_core::{ControlSize, FlexDirection, FlexWrap, LengthSpec};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, LayoutBox,
    MutationQueue, NodeKind, NodeStyle, PointerDragPhase, StableNodeId, UiWorld,
};

const DRAG_THRESHOLD: f32 = nana_ui_core::space::XS;
const DEFAULT_SPACING: f32 = nana_ui_core::space::XXS;
const INSERT_INSET: f32 = nana_ui_core::space::XS;
const INSERT_THICKNESS: f32 = nana_ui_core::space::XXS;
const DEFAULT_INDENT: f32 = nana_ui_core::space::LG;

/// The `z-index` a row stacks at while it follows the pointer in a live,
/// flat drag ([`ReorderDragVisual::preview`]): over the rows it passes, under
/// an open menu, which stacks at [`crate::MENU_OVERLAY_Z_INDEX`].
pub const REORDER_PREVIEW_Z_INDEX: i32 = 100;
const _: () = assert!(REORDER_PREVIEW_Z_INDEX < crate::popover::MENU_OVERLAY_Z_INDEX);

pub use crate::tree_view::{TreeDropIntent, TreeDropPosition};

/// How a [`ReorderList`] lays out its rows and resolves drop coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReorderLayout {
    /// Vertical column of rows.
    #[default]
    Column,
    /// Wrapping left-to-right grid of tiles (`FlexDirection::Row` + `FlexWrap::Wrap`).
    Grid,
}

/// How an active drag presents its landing target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReorderDragPreview {
    /// Flat live lists preview on the presentation layer; tree drops keep marks.
    #[default]
    Auto,
    /// Unified live gap-opening presentation with dashed landing slot and
    /// indented nested / parent ring feedback across flat, tree, cross-group,
    /// and 2D grid lists.
    LiveGap,
    /// Always show insert line / outline marks without shifting rows.
    Marks,
}

/// Pointer payload fed by an external drag (such as another window or a drawer)
/// into [`AppContext::set_reorder_external_drag`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReorderExternalDrag {
    pub value: Arc<str>,
    pub kind: Option<Arc<str>>,
    /// Pointer coordinates in window space.
    pub x: f32,
    pub y: f32,
    /// Dimensions `[width, height]` of the carried item for empty/wrapping slots.
    pub size: Option<[f32; 2]>,
}

/// Resolved presentation shifts and landing indicator for a [`ReorderDragVisual`]
/// in [`ReorderDragPreview::LiveGap`] mode.
#[derive(Debug, Clone, PartialEq)]
pub struct ReorderGapPlan {
    /// Per-row presentation offsets `[dx, dy]`.
    pub shifts: Vec<[f32; 2]>,
    /// Dashed rounded slot box in the evaluated coordinate frame.
    pub slot: Option<LayoutBox>,
    /// Whether `slot` is nested inside `inside` and needs a tree connector.
    pub slot_nested: bool,
    /// Highlighted parent box, shifted by its own presentation offset.
    pub inside: Option<LayoutBox>,
}

/// Results published by [`ReorderList`]. Order and persistence stay with the
/// application.
#[derive(Debug, Clone, PartialEq)]
pub enum ReorderListEvent {
    Select(Arc<str>),
    /// Secondary (right) press resolved to a row body. Row surfaces project
    /// with `pointer_events: none` (the list owns drag and selection), so the
    /// hit node above the row can never reach a row handler while bubbling;
    /// [`AppContext::secondary_press_at`] resolves the row under the point and
    /// publishes this event on the list instead. `x`/`y` are the press point
    /// in window coordinates, ready to anchor a context menu.
    Secondary {
        source: Arc<str>,
        x: f32,
        y: f32,
    },
    /// Lifecycle updates published during an active [`ReorderDragPreview::LiveGap`]
    /// drag so outer containers (dock buttons, collapsed folders, or cross-window
    /// drags) can track the pointer in window coordinates.
    Drag {
        source: Arc<str>,
        phase: PointerDragPhase,
        x: f32,
        y: f32,
        inside_group: bool,
    },
    Reorder {
        source: Arc<str>,
        before: Option<Arc<str>>,
    },
    TreeDrop {
        source: Arc<str>,
        intent: TreeDropIntent,
    },
    Cancelled,
}

/// Pointer phases consumed by [`ReorderList::apply_pointer`].
///
/// Escape, unfocus, and lost-touch should be delivered as [`Self::Cancel`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReorderListPointer {
    Down { x: f32, y: f32 },
    Move { x: f32, y: f32 },
    Up { x: f32, y: f32 },
    Cancel,
}

/// Which rows a drop place takes, by their [`ReorderItem::kind`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ReorderAccepts {
    /// Every row, with a kind or without.
    #[default]
    All,
    /// No row.
    Nothing,
    /// Rows of these kinds. A row without a kind is none of them.
    Kinds(Arc<[Arc<str>]>),
}

impl ReorderAccepts {
    pub fn kinds<K: Into<Arc<str>>>(kinds: impl IntoIterator<Item = K>) -> Self {
        Self::Kinds(kinds.into_iter().map(Into::into).collect())
    }

    /// Whether a row of `kind` may land here.
    pub fn accepts(&self, kind: Option<&str>) -> bool {
        match self {
            Self::All => true,
            Self::Nothing => false,
            Self::Kinds(kinds) => kind.is_some_and(|kind| kinds.iter().any(|k| &**k == kind)),
        }
    }
}

/// One row identity. A press on a control inside a live row (a focusable
/// node below the row, such as a button) is the control's click unless the
/// pointer moves past the drag threshold, when the row drags instead.
/// Optional [`Self::tools`] is a live child that keeps the pointer to itself:
/// hits there never start a drag.
///
/// For a tree, rows come in depth-first order and
/// [`depth`](field@Self::depth) says how deep each sits: a row holds the
/// deeper rows right below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReorderItem {
    pub value: Arc<str>,
    pub label: Arc<str>,
    pub draggable: bool,
    /// The row moves when dragged. A row that does not still selects on a
    /// click and takes drops; dragging it does nothing.
    pub movable: bool,
    pub drop_target: bool,
    pub selected: bool,
    pub disabled: bool,
    /// How deep the row sits; 0 at the top level.
    pub depth: u16,
    /// What the row is, for the [`ReorderAccepts`] of the places it may go.
    pub kind: Option<Arc<str>>,
    /// Which rows a tree drop may put inside this one. All by default.
    pub accepts_inside: ReorderAccepts,
    /// Which rows may land right before or after this one, beside it in its
    /// parent. All by default; the parent must take them inside too.
    pub accepts_sibling: ReorderAccepts,
    /// When `true` in a tree list with [`ReorderDragPreview::LiveGap`],
    /// hovering the middle zone highlights the row as a folded container
    /// without opening an indented child slot underneath it.
    pub folded: bool,
    pub tools: Option<StableNodeId>,
}

impl ReorderItem {
    pub fn new(value: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            draggable: true,
            movable: true,
            drop_target: true,
            selected: false,
            disabled: false,
            depth: 0,
            kind: None,
            accepts_inside: ReorderAccepts::All,
            accepts_sibling: ReorderAccepts::All,
            folded: false,
            tools: None,
        }
    }

    /// Sets whether this row can start a drag. Also sets [`Self::drop_target`]
    /// to the same value; call [`Self::drop_target`] afterwards for drop-only
    /// rows.
    pub fn draggable(mut self, draggable: bool) -> Self {
        self.draggable = draggable;
        self.drop_target = draggable;
        self
    }

    /// See [`Self::movable`](field@Self::movable).
    pub fn movable(mut self, movable: bool) -> Self {
        self.movable = movable;
        self
    }

    pub fn drop_target(mut self, drop_target: bool) -> Self {
        self.drop_target = drop_target;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// See [`Self::depth`](field@Self::depth).
    pub fn depth(mut self, depth: u16) -> Self {
        self.depth = depth;
        self
    }

    /// See [`Self::kind`](field@Self::kind).
    pub fn kind(mut self, kind: impl Into<Arc<str>>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    /// See [`Self::accepts_inside`](field@Self::accepts_inside).
    /// [`ReorderAccepts::Nothing`] splits the row into before and after
    /// halves.
    pub fn accepts_inside(mut self, accepts: ReorderAccepts) -> Self {
        self.accepts_inside = accepts;
        self
    }

    /// See [`Self::accepts_sibling`](field@Self::accepts_sibling).
    pub fn accepts_sibling(mut self, accepts: ReorderAccepts) -> Self {
        self.accepts_sibling = accepts;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// See [`Self::folded`](field@Self::folded).
    pub fn folded(mut self, folded: bool) -> Self {
        self.folded = folded;
        self
    }

    /// A region that never drags: pointer hits inside this child's layout box
    /// do not begin a reorder gesture, even once they move. Buttons do not
    /// need it to keep their clicks; it is for a part of the row that must
    /// never move the row.
    pub fn tools(mut self, tools: StableNodeId) -> Self {
        self.tools = Some(tools);
        self
    }

    fn is_source(&self) -> bool {
        self.draggable && !self.disabled
    }

    fn is_drop_target(&self) -> bool {
        self.drop_target && !self.disabled
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ReorderDrag {
    source: Arc<str>,
    start_x: f32,
    start_y: f32,
    x: f32,
    y: f32,
    moved: bool,
    /// Another list of the [`ReorderList::transfer_group`] the pointer is
    /// over; it shows and takes the drop instead of this list.
    over: Option<StableNodeId>,
    /// Pointer is outside this list's zone and not over a sibling transfer list.
    outside_group: bool,
    /// `ReorderListEvent::Drag { phase: Started, .. }` has been emitted.
    drag_emitted: bool,
    /// Border-box origin `[bounds.x, bounds.y]` in page coordinates at pointer update time.
    origin: Option<[f32; 2]>,
}

/// A row of another list of the same [`ReorderList::transfer_group`] dragged
/// over this one. The source list keeps the pointer and the gesture; this
/// list only resolves and shows where the row would land among its own rows.
#[derive(Debug, Clone, PartialEq)]
struct IncomingDrag {
    /// The dragged row's value in the list it comes from.
    value: Arc<str>,
    kind: Option<Arc<str>>,
    /// The pointer, in this list's page layout coordinates.
    x: f32,
    y: f32,
    /// Border-box origin `[bounds.x, bounds.y]` in page coordinates.
    origin: Option<[f32; 2]>,
    /// Dimensions `[width, height]` of the carried item.
    carried_size: Option<[f32; 2]>,
    /// Row-to-row pitch (height + gap) of the carried item.
    carried_step: Option<f32>,
}

/// What a list shows while a row is dragged past the threshold: which row,
/// where the pointer is and which rows take a drop. The row boxes come from
/// layout — the list's own uniform rows, or its live row children — so the
/// drop indicator is resolved where those boxes are known.
#[derive(Debug, Clone, PartialEq)]
pub struct ReorderDragVisual {
    /// The dragged row's index in [`ReorderList::items`]; `None` for a row
    /// dragged in from another list of the
    /// [`ReorderList::transfer_group`], which has no row here.
    pub source: Option<usize>,
    pub x: f32,
    pub y: f32,
    /// Where the press that started the drag landed, in the same layout
    /// coordinates as [`Self::x`] and [`Self::y`].
    pub press_x: f32,
    pub press_y: f32,
    pub tree_drop: bool,
    /// Live rows in a drag: the carried rows follow the pointer and the
    /// remaining rows shift on the presentation layer over unchanged layout.
    pub preview: bool,
    /// Unified live gap-opening mode ([`ReorderDragPreview::LiveGap`]): draws
    /// dashed landing slots and (for open tree parents) parent highlight +
    /// L-connector rather than solid insert lines and source outlines.
    pub live_gap: bool,
    /// Active layout mode ([`ReorderLayout::Column`] or [`ReorderLayout::Grid`]).
    pub layout_mode: ReorderLayout,
    rules: Arc<DropRules>,
    carried: Carried,
    /// The pointer is over another list of the transfer group, which shows
    /// the drop: this list marks none of its own.
    elsewhere: bool,
    /// Pointer is outside this list's drop zone and not over a sibling list.
    outside_group: bool,
    /// List origin `[bounds.x, bounds.y]` in page coordinates at capture time.
    origin: Option<[f32; 2]>,
    /// Dimensions `[width, height]` for foreign drags.
    carried_size: Option<[f32; 2]>,
    /// Pitch for foreign drags.
    carried_step: Option<f32>,
}

/// Where a drop would land, as drawn: a line between rows, or the row a
/// tree drop goes inside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReorderDropMark {
    Line(LayoutBox),
    Inside(LayoutBox),
}

impl ReorderDragVisual {
    /// How many rows the list has.
    pub fn rows(&self) -> usize {
        self.rules.rows.len()
    }

    /// The rows the drag carries: the dragged row and the rows it holds.
    /// `None` for a row dragged in from another list.
    pub fn carried(&self) -> Option<std::ops::RangeInclusive<usize>> {
        self.rules.span(&self.carried)
    }

    /// The box around the carried rows, among `rows`.
    pub fn carried_box(&self, rows: &[LayoutBox]) -> Option<LayoutBox> {
        let carried = self.carried()?;
        let first = rows.get(*carried.start())?;
        let last = rows.get(*carried.end()).unwrap_or(first);
        let x = first.x.min(last.x);
        let right = (first.x + first.width).max(last.x + last.width);
        Some(LayoutBox {
            x,
            y: first.y,
            width: right - x,
            height: last.y + last.height - first.y,
        })
    }

    /// The drop indicator over `rows` inside `bounds`: the insert line, or
    /// the target's line for a tree drop; `None` where a drop changes nothing.
    pub fn insert_line(&self, bounds: LayoutBox, rows: &[LayoutBox]) -> Option<LayoutBox> {
        self.drop_mark(bounds, rows).map(|mark| match mark {
            ReorderDropMark::Line(line) | ReorderDropMark::Inside(line) => line,
        })
    }

    fn pointer_in(&self, bounds: LayoutBox) -> (f32, f32) {
        match self.origin {
            Some([ox, oy]) => (self.x - ox + bounds.x, self.y - oy + bounds.y),
            None => (self.x, self.y),
        }
    }

    /// [`Self::insert_line`], telling a line from a row a drop goes inside.
    /// A tree drop's line is indented to the depth the row lands at.
    /// Nothing while the pointer is over another list of the transfer group,
    /// which marks the drop itself.
    pub fn drop_mark(&self, bounds: LayoutBox, rows: &[LayoutBox]) -> Option<ReorderDropMark> {
        if self.elsewhere || self.outside_group {
            return None;
        }
        let (px, py) = self.pointer_in(bounds);
        if self.tree_drop {
            let drop = self.rules.tree_drop(rows, &self.carried, px, py)?;
            let indent = f32::from(self.rules.rows[drop.target].depth) * self.rules.indent;
            let mark = tree_insert_line(rows[drop.anchor], drop.position, indent);
            return Some(if drop.position == TreeDropPosition::Inside {
                ReorderDropMark::Inside(mark)
            } else {
                ReorderDropMark::Line(mark)
            });
        }
        let before = if self.rules.layout_mode == ReorderLayout::Grid {
            self.rules.grid_drop(rows, &self.carried, px, py)?
        } else {
            self.rules.flat_drop(rows, &self.carried, py)?
        };
        Some(ReorderDropMark::Line(reorder_insert_line(
            bounds, rows, before,
        )))
    }

    /// Resolves presentation shifts (`[dx, dy]` per row) and the dashed landing
    /// slot / nested parent highlight for [`ReorderDragPreview::LiveGap`].
    pub fn gap_plan(&self, bounds: LayoutBox, boxes: &[LayoutBox]) -> Option<ReorderGapPlan> {
        if !self.live_gap || boxes.len() != self.rows() {
            return None;
        }
        if self.elsewhere {
            let mut shifts = vec![[0.0, 0.0]; boxes.len()];
            if let Some(carried) = self.carried() {
                let (start, end) = (*carried.start(), *carried.end());
                if end < boxes.len() {
                    if self.rules.layout_mode == ReorderLayout::Grid && !self.tree_drop {
                        let count = end - start + 1;
                        for i in (end + 1)..boxes.len() {
                            let dst = boxes[i - count];
                            shifts[i] = [dst.x - boxes[i].x, dst.y - boxes[i].y];
                        }
                    } else {
                        let block = carried_block_step(boxes, start, end, self.rules.spacing);
                        shifts[end + 1..].fill([0.0, -block]);
                    }
                }
            }
            return Some(ReorderGapPlan {
                shifts,
                slot: None,
                slot_nested: false,
                inside: None,
            });
        }
        if self.outside_group {
            return Some(ReorderGapPlan {
                shifts: vec![[0.0, 0.0]; boxes.len()],
                slot: self.carried_box(boxes),
                slot_nested: false,
                inside: None,
            });
        }
        let (px, py) = self.pointer_in(bounds);
        let spacing = self.rules.spacing.max(0.0);
        if self.tree_drop {
            let mut shifts = vec![[0.0, 0.0]; boxes.len()];
            let drop = self.rules.tree_drop(boxes, &self.carried, px, py);
            let Some(drop) = drop else {
                let fallback_slot = self.carried_box(boxes).map(|rect| {
                    let depth = self.source.map(|s| self.rules.depth(s)).unwrap_or(0);
                    let indent_px =
                        (f32::from(depth) * self.rules.indent).min((rect.width - 24.0).max(0.0));
                    LayoutBox {
                        x: rect.x + indent_px,
                        width: (rect.width - indent_px).max(0.0),
                        ..rect
                    }
                });
                return Some(ReorderGapPlan {
                    shifts,
                    slot: fallback_slot,
                    slot_nested: false,
                    inside: None,
                });
            };
            if drop.position == TreeDropPosition::Inside && self.rules.rows[drop.target].folded {
                return Some(ReorderGapPlan {
                    shifts,
                    slot: None,
                    slot_nested: false,
                    inside: Some(boxes[drop.target]),
                });
            }
            let (insert_idx, depth, slot_nested, parent) = match drop.position {
                TreeDropPosition::Before => {
                    (drop.target, self.rules.depth(drop.target), false, None)
                }
                TreeDropPosition::After => (
                    self.rules.subtree_end(drop.target) + 1,
                    self.rules.depth(drop.target),
                    false,
                    None,
                ),
                TreeDropPosition::Inside => (
                    drop.target + 1,
                    self.rules.depth(drop.target).saturating_add(1),
                    true,
                    Some(drop.target),
                ),
            };
            let (slot_y, slot_h) = match self.carried() {
                Some(carried) => {
                    let (start, end) = (*carried.start(), *carried.end());
                    let slot_h = self
                        .carried_box(boxes)
                        .map_or_else(|| boxes.get(start).map_or(32.0, |b| b.height), |b| b.height);
                    let block = carried_block_step(boxes, start, end, spacing);
                    let slot_y = if insert_idx > end {
                        shifts[end + 1..insert_idx].fill([0.0, -block]);
                        if insert_idx < boxes.len() {
                            boxes[insert_idx].y - block
                        } else if let Some(prev) = boxes.get(insert_idx.saturating_sub(1)) {
                            prev.y + prev.height + spacing - block
                        } else {
                            bounds.y
                        }
                    } else if insert_idx <= start {
                        shifts[insert_idx..start].fill([0.0, block]);
                        boxes.get(insert_idx).map_or(bounds.y, |b| b.y)
                    } else {
                        boxes[start].y
                    };
                    (slot_y, slot_h)
                }
                None => {
                    let default_h = boxes.first().map_or(32.0, |b| b.height);
                    let slot_h = self.carried_size.map_or(default_h, |s| s[1]);
                    let block = self.carried_step.unwrap_or(slot_h + spacing);
                    shifts[insert_idx..].fill([0.0, block]);
                    let slot_y = boxes
                        .get(insert_idx)
                        .map(|b| b.y)
                        .or_else(|| boxes.last().map(|b| b.y + b.height + spacing))
                        .unwrap_or(bounds.y);
                    (slot_y, slot_h)
                }
            };
            let indent_px =
                (f32::from(depth) * self.rules.indent).min((bounds.width - 24.0).max(0.0));
            let slot = Some(LayoutBox {
                x: bounds.x + indent_px,
                y: slot_y,
                width: (bounds.width - indent_px).max(0.0),
                height: slot_h,
            });
            let inside = parent.map(|p| LayoutBox {
                y: boxes[p].y + shifts[p][1],
                ..boxes[p]
            });
            return Some(ReorderGapPlan {
                shifts,
                slot,
                slot_nested,
                inside,
            });
        }
        if self.rules.layout_mode == ReorderLayout::Grid {
            let mut shifts = vec![[0.0, 0.0]; boxes.len()];
            let before = self.rules.grid_drop(boxes, &self.carried, px, py);
            if boxes.is_empty() {
                let slot = before.map(|_| {
                    let [w, h] = self.carried_size.unwrap_or([bounds.width.min(96.0), 76.0]);
                    LayoutBox {
                        x: bounds.x,
                        y: bounds.y,
                        width: w,
                        height: h,
                    }
                });
                return Some(ReorderGapPlan {
                    shifts,
                    slot,
                    slot_nested: false,
                    inside: None,
                });
            }
            let slot_at = |idx: usize| -> LayoutBox {
                if let Some(b) = boxes.get(idx) {
                    return *b;
                }
                let origin_x = boxes[0].x;
                let max_right = bounds.x + bounds.width + 0.5;
                let col_gap = self.rules.column_gap.max(0.0);
                let row_gap = self.rules.spacing.max(0.0);
                let mut cur = *boxes.last().unwrap();
                for _ in boxes.len()..=idx {
                    let next_x = cur.x + cur.width + col_gap;
                    cur = if next_x + cur.width <= max_right {
                        LayoutBox { x: next_x, ..cur }
                    } else {
                        LayoutBox {
                            x: origin_x,
                            y: cur.y + cur.height + row_gap,
                            ..cur
                        }
                    };
                }
                cur
            };
            let slot = match (&self.carried, before) {
                (Carried::Row(source), None) => Some(boxes[*source]),
                (Carried::Row(source), Some(before)) => {
                    let raw = before.unwrap_or(boxes.len());
                    let target_pos = if raw > *source { raw - 1 } else { raw };
                    if target_pos > *source {
                        for i in (*source + 1)..=target_pos {
                            let dst = boxes[i - 1];
                            shifts[i] = [dst.x - boxes[i].x, dst.y - boxes[i].y];
                        }
                    } else if target_pos < *source {
                        for i in target_pos..*source {
                            let dst = boxes[i + 1];
                            shifts[i] = [dst.x - boxes[i].x, dst.y - boxes[i].y];
                        }
                    }
                    Some(boxes[target_pos])
                }
                (Carried::Foreign(_), None) => None,
                (Carried::Foreign(_), Some(before)) => {
                    let idx = before.unwrap_or(boxes.len());
                    for i in idx..boxes.len() {
                        let dst = slot_at(i + 1);
                        shifts[i] = [dst.x - boxes[i].x, dst.y - boxes[i].y];
                    }
                    Some(slot_at(idx))
                }
            };
            return Some(ReorderGapPlan {
                shifts,
                slot,
                slot_nested: false,
                inside: None,
            });
        }
        let mut shifts = vec![[0.0, 0.0]; boxes.len()];
        let before = self.rules.flat_drop(boxes, &self.carried, py);
        if boxes.is_empty() {
            let slot = before.map(|_| LayoutBox {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: self.carried_size.map_or(32.0, |s| s[1]),
            });
            return Some(ReorderGapPlan {
                shifts,
                slot,
                slot_nested: false,
                inside: None,
            });
        }
        let slot = match (self.carried(), before) {
            (Some(_), None) => self.carried_box(boxes),
            (Some(carried), Some(before)) => {
                let (start, end) = (*carried.start(), *carried.end());
                let slot_h = self
                    .carried_box(boxes)
                    .map_or(boxes[start].height, |b| b.height);
                let block = carried_block_step(boxes, start, end, spacing);
                let idx = before.unwrap_or(boxes.len());
                let slot_y = if idx > end {
                    shifts[end + 1..idx].fill([0.0, -block]);
                    if idx < boxes.len() {
                        boxes[idx].y - block
                    } else {
                        boxes[idx - 1].y + boxes[idx - 1].height + spacing - block
                    }
                } else {
                    shifts[idx..start].fill([0.0, block]);
                    boxes[idx].y
                };
                let rect = self.carried_box(boxes).unwrap_or(boxes[start]);
                Some(LayoutBox {
                    x: rect.x,
                    y: slot_y,
                    width: rect.width,
                    height: slot_h,
                })
            }
            (None, None) => None,
            (None, Some(before)) => {
                let idx = before.unwrap_or(boxes.len());
                let slot_h = self.carried_size.map_or(boxes[0].height, |s| s[1]);
                let block = self.carried_step.unwrap_or(slot_h + spacing);
                shifts[idx..].fill([0.0, block]);
                let slot_y = boxes
                    .get(idx)
                    .map(|b| b.y)
                    .or_else(|| boxes.last().map(|b| b.y + b.height + spacing))
                    .unwrap_or(bounds.y);
                Some(LayoutBox {
                    x: boxes[0].x,
                    y: slot_y,
                    width: boxes[0].width,
                    height: slot_h,
                })
            }
        };
        Some(ReorderGapPlan {
            shifts,
            slot,
            slot_nested: false,
            inside: None,
        })
    }

    /// How far the carried rows have been moved from where they were
    /// pressed, `[x, y]`.
    pub fn preview_offset(&self) -> [f32; 2] {
        [self.x - self.press_x, self.y - self.press_y]
    }

    /// How far each of `rows` moves down (up when negative) to make room for
    /// the carried rows where they would land: the rows between their place
    /// and that spot shift by the carried block's height, its spacing
    /// included. All zero where the drop changes nothing or may not land,
    /// and for the carried rows themselves, which follow the pointer; all
    /// zero too while the pointer is over another list, or for a row dragged
    /// in from one.
    pub fn shift_offsets(&self, rows: &[LayoutBox]) -> Vec<f32> {
        let mut offsets = vec![0.0; rows.len()];
        let Some(carried) = self.carried().filter(|_| !self.elsewhere) else {
            return offsets;
        };
        let (start, end) = (*carried.start(), *carried.end());
        if rows.len() != self.rows() || end >= rows.len() {
            return offsets;
        }
        let Some(before) = self.rules.flat_drop(rows, &self.carried, self.y) else {
            return offsets;
        };
        // The carried block's height with one gap: from its top to the next
        // row's top, or from the bottom of the row above it to its bottom.
        let block = match (rows.get(end + 1), start.checked_sub(1)) {
            (Some(next), _) => next.y - rows[start].y,
            (None, Some(above)) => {
                rows[end].y + rows[end].height - (rows[above].y + rows[above].height)
            }
            (None, None) => 0.0,
        };
        if !block.is_finite() {
            return offsets;
        }
        let before = before.unwrap_or(rows.len());
        if before > end {
            offsets[end + 1..before].fill(-block);
        } else {
            offsets[before..start].fill(block);
        }
        offsets
    }
}

/// A keyboard move of the chosen row, for [`ReorderList::key_move`]. Alt
/// with an arrow key on a focused list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReorderKeyMove {
    /// Before its previous sibling.
    Up,
    /// After its next sibling.
    Down,
    /// Into its previous sibling, as that row's last child.
    Indent,
    /// Out of its parent, right after it.
    Outdent,
}

/// What a row is and takes, for drop resolution.
#[derive(Debug, Clone, PartialEq)]
struct DropRow {
    depth: u16,
    target: bool,
    kind: Option<Arc<str>>,
    inside: ReorderAccepts,
    sibling: ReorderAccepts,
    folded: bool,
}

/// What a drag carries over a list's rows.
#[derive(Debug, Clone, PartialEq)]
enum Carried {
    /// This row of the list, and the rows it holds.
    Row(usize),
    /// A row of this kind dragged in from another list of the transfer
    /// group: no row here moves with it, and anywhere it may land is new.
    Foreign(Option<Arc<str>>),
}

/// A list's rows as a flattened tree, and where a dragged subtree may land.
#[derive(Debug, Clone, PartialEq)]
struct DropRules {
    rows: Vec<DropRow>,
    root: ReorderAccepts,
    indent: f32,
    spacing: f32,
    column_gap: f32,
    layout_mode: ReorderLayout,
}

/// A resolved tree drop: `position` of `target`, drawn against `anchor`
/// (the row under the pointer for a line after a deeper row).
#[derive(Debug, Clone, Copy, PartialEq)]
struct TreeDrop {
    target: usize,
    position: TreeDropPosition,
    anchor: usize,
}

fn carried_block_step(rows: &[LayoutBox], start: usize, end: usize, spacing: f32) -> f32 {
    let block = match (rows.get(end + 1), start.checked_sub(1)) {
        (Some(next), _) => next.y - rows[start].y,
        (None, Some(above)) => {
            rows[end].y + rows[end].height - (rows[above].y + rows[above].height)
        }
        (None, None) => (rows[end].y + rows[end].height - rows[start].y) + spacing.max(0.0),
    };
    if block.is_finite() {
        block.max(0.0)
    } else {
        0.0
    }
}

impl DropRules {
    fn of(list: &ReorderList) -> Self {
        Self {
            rows: list
                .items
                .iter()
                .map(|item| DropRow {
                    depth: item.depth,
                    target: item.is_drop_target(),
                    kind: item.kind.clone(),
                    inside: item.accepts_inside.clone(),
                    sibling: item.accepts_sibling.clone(),
                    folded: item.folded,
                })
                .collect(),
            root: list.accepts_root.clone(),
            indent: list.indent,
            spacing: list.spacing,
            column_gap: list.column_gap.unwrap_or(list.spacing).max(0.0),
            layout_mode: list.layout_mode,
        }
    }

    fn depth(&self, index: usize) -> u16 {
        self.rows[index].depth
    }

    /// The kind of what `carried` is.
    fn kind<'a>(&'a self, carried: &'a Carried) -> Option<&'a str> {
        match carried {
            Carried::Row(source) => self.rows[*source].kind.as_deref(),
            Carried::Foreign(kind) => kind.as_deref(),
        }
    }

    /// The rows of this list `carried` moves, if any.
    fn span(&self, carried: &Carried) -> Option<std::ops::RangeInclusive<usize>> {
        match carried {
            Carried::Row(source) => Some(*source..=self.subtree_end(*source)),
            Carried::Foreign(_) => None,
        }
    }

    /// Whether row `index` moves with `carried`.
    fn holds(&self, carried: &Carried, index: usize) -> bool {
        self.span(carried).is_some_and(|span| span.contains(&index))
    }

    /// The last row `index` holds, or `index` itself.
    fn subtree_end(&self, index: usize) -> usize {
        let depth = self.depth(index);
        let mut end = index;
        while self.rows.get(end + 1).is_some_and(|row| row.depth > depth) {
            end += 1;
        }
        end
    }

    fn parent(&self, index: usize) -> Option<usize> {
        let depth = self.depth(index);
        (0..index).rev().find(|&row| self.depth(row) < depth)
    }

    fn previous_sibling(&self, index: usize) -> Option<usize> {
        let depth = self.depth(index);
        (0..index)
            .rev()
            .find(|&row| self.depth(row) <= depth)
            .filter(|&row| self.depth(row) == depth)
    }

    fn next_sibling(&self, index: usize) -> Option<usize> {
        let next = self.subtree_end(index) + 1;
        (next < self.rows.len() && self.depth(next) == self.depth(index)).then_some(next)
    }

    /// The row at `depth` that holds `index`, or `index` itself.
    fn ancestor_at(&self, index: usize, depth: u16) -> Option<usize> {
        (0..=index)
            .rev()
            .find(|&row| self.depth(row) <= depth)
            .filter(|&row| self.depth(row) == depth)
    }

    fn takes_child(&self, parent: Option<usize>, kind: Option<&str>) -> bool {
        match parent {
            Some(parent) => self.rows[parent].inside.accepts(kind),
            None => self.root.accepts(kind),
        }
    }

    /// Whether `source` and the rows it holds may go to `position` of
    /// `target`: never into themselves, only onto a drop target, inside a
    /// row that takes their kind, or beside one whose parent does.
    fn allows(&self, carried: &Carried, target: usize, position: TreeDropPosition) -> bool {
        if self.holds(carried, target) || !self.rows[target].target {
            return false;
        }
        let kind = self.kind(carried);
        match position {
            TreeDropPosition::Inside => self.rows[target].inside.accepts(kind),
            TreeDropPosition::Before | TreeDropPosition::After => {
                self.rows[target].sibling.accepts(kind)
                    && self.takes_child(self.parent(target), kind)
            }
        }
    }

    /// Whether the move puts `carried` anywhere new.
    fn changes(&self, carried: &Carried, target: usize, position: TreeDropPosition) -> bool {
        let Carried::Row(source) = *carried else {
            return true;
        };
        match position {
            TreeDropPosition::Before => self.next_sibling(source) != Some(target),
            TreeDropPosition::After => self.previous_sibling(source) != Some(target),
            TreeDropPosition::Inside => true,
        }
    }

    /// The first of `places` that `carried` may go to, if it moves it.
    fn first_allowed(
        &self,
        carried: &Carried,
        places: impl IntoIterator<Item = TreeDrop>,
    ) -> Option<TreeDrop> {
        places
            .into_iter()
            .find(|drop| self.allows(carried, drop.target, drop.position))
            .filter(|drop| self.changes(carried, drop.target, drop.position))
    }

    /// Where a tree drop of `carried` at `(x, y)` over `boxes` lands. The top
    /// quarter of a row is before it, the middle inside (halves without
    /// inside, for a row that does not take the carried kind), the bottom
    /// after it — or, over a row whose children show below it, before its
    /// first child. After the last row of nested rows, `x` picks the depth:
    /// each [`ReorderList::indent`] left of the row's own is one level out.
    fn tree_drop(
        &self,
        boxes: &[LayoutBox],
        carried: &Carried,
        x: f32,
        y: f32,
    ) -> Option<TreeDrop> {
        let hovered = boxes
            .iter()
            .take(self.rows.len())
            .position(|row| row.contains(x, y))
            .or_else(|| {
                let tol = self.spacing.max(0.0) * 0.5 + 0.5;
                boxes
                    .iter()
                    .take(self.rows.len())
                    .enumerate()
                    .filter(|(_, row)| {
                        x >= row.x
                            && x <= row.x + row.width
                            && y >= row.y - tol
                            && y <= row.y + row.height + tol
                    })
                    .min_by(|(_, a), (_, b)| {
                        let da = ((a.y + a.height * 0.5) - y).abs();
                        let db = ((b.y + b.height * 0.5) - y).abs();
                        da.total_cmp(&db)
                    })
                    .map(|(index, _)| index)
            })?;
        if self.holds(carried, hovered) {
            return None;
        }
        let row = boxes[hovered];
        let offset = (y - row.y) / row.height.max(1.0);
        let inside = self.rows[hovered].inside.accepts(self.kind(carried));
        let at = |target, position, anchor| TreeDrop {
            target,
            position,
            anchor,
        };
        if offset < if inside { 0.25 } else { 0.5 } {
            return self.first_allowed(carried, [at(hovered, TreeDropPosition::Before, hovered)]);
        }
        if inside && offset <= 0.75 {
            return self.first_allowed(carried, [at(hovered, TreeDropPosition::Inside, hovered)]);
        }
        // The row after, past what the drag carries.
        let next = match self.span(carried) {
            Some(span) if hovered + 1 == *span.start() => span.end() + 1,
            _ => hovered + 1,
        };
        let depth = self.depth(hovered);
        if next < self.rows.len() && self.depth(next) > depth {
            return self.first_allowed(
                carried,
                [
                    at(next, TreeDropPosition::Before, next),
                    at(hovered, TreeDropPosition::Inside, hovered),
                ],
            );
        }
        let floor = if next < self.rows.len() {
            self.depth(next)
        } else {
            0
        };
        let wanted = if self.indent > 0.0 {
            ((x - row.x - INSERT_INSET) / self.indent).floor().max(0.0) as u16
        } else {
            depth
        };
        let wanted = wanted.clamp(floor, depth);
        self.first_allowed(
            carried,
            (floor..=wanted).rev().filter_map(|level| {
                self.ancestor_at(hovered, level)
                    .map(|target| at(target, TreeDropPosition::After, hovered))
            }),
        )
    }

    /// The row a flat drop of `carried` at `y` goes before (`None` for the
    /// end); `None` overall where it changes nothing or may not land.
    fn flat_drop(&self, boxes: &[LayoutBox], carried: &Carried, y: f32) -> Option<Option<usize>> {
        let kind = self.kind(carried);
        if self.rows.is_empty() {
            return (!matches!(carried, Carried::Row(_)) && self.root.accepts(kind))
                .then_some(None);
        }
        let before = boxes
            .iter()
            .take(self.rows.len())
            .enumerate()
            .find_map(|(index, row)| {
                (!self.holds(carried, index)
                    && self.rows[index].target
                    && y < row.y + row.height * 0.5)
                    .then_some(index)
            });
        let unchanged = self.span(carried).is_some_and(|span| match before {
            Some(before) => before == span.end() + 1,
            None => span.end() + 1 == self.rows.len(),
        });
        let accepted = match before {
            Some(before) => self.rows[before].target && self.rows[before].sibling.accepts(kind),
            None => self.root.accepts(kind),
        };
        (!unchanged && accepted).then_some(before)
    }

    /// The item a 2D grid drop of `carried` at `(x, y)` goes before (`None`
    /// for the end); `None` overall where it changes nothing or may not land.
    fn grid_drop(
        &self,
        boxes: &[LayoutBox],
        carried: &Carried,
        x: f32,
        y: f32,
    ) -> Option<Option<usize>> {
        let kind = self.kind(carried);
        let count = boxes.len().min(self.rows.len());
        if count == 0 {
            return (!matches!(carried, Carried::Row(_)) && self.root.accepts(kind))
                .then_some(None);
        }
        let mut visual_rows: Vec<(usize, usize, f32, f32)> = Vec::new();
        for (index, item_box) in boxes.iter().take(count).enumerate() {
            let bottom = item_box.y + item_box.height;
            if let Some(last) = visual_rows.last_mut()
                && (item_box.y - last.2).abs() <= item_box.height.max(1.0) * 0.5
            {
                last.1 = index;
                last.2 = last.2.min(item_box.y);
                last.3 = last.3.max(bottom);
            } else {
                visual_rows.push((index, index, item_box.y, bottom));
            }
        }
        let row_gap_half = self.spacing.max(0.0) * 0.5;
        let last_row = *visual_rows.last()?;
        let before = if y > last_row.3 + row_gap_half {
            None
        } else {
            let row_idx = visual_rows
                .iter()
                .position(|&(_, _, top, bottom)| {
                    y >= top - row_gap_half && y <= bottom + row_gap_half
                })
                .unwrap_or_else(|| {
                    visual_rows
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| {
                            let ca = (a.2 + a.3) * 0.5;
                            let cb = (b.2 + b.3) * 0.5;
                            (ca - y).abs().total_cmp(&(cb - y).abs())
                        })
                        .map_or(0, |(idx, _)| idx)
                });
            let (first_idx, last_idx, _, _) = visual_rows[row_idx];
            let found = (first_idx..=last_idx).find(|&idx| {
                (self.rows[idx].target || self.holds(carried, idx))
                    && x < boxes[idx].x + boxes[idx].width * 0.5
            });
            match found {
                Some(idx) => Some(idx),
                None => {
                    let after = last_idx + 1;
                    (after < count).then_some(after)
                }
            }
        };
        let unchanged = match *carried {
            Carried::Row(source) => {
                before == Some(source)
                    || before == Some(source + 1)
                    || (before.is_none() && source + 1 == count)
            }
            Carried::Foreign(_) => false,
        };
        let accepted = match before {
            Some(before) => {
                (self.rows[before].target || self.holds(carried, before))
                    && self.rows[before].sibling.accepts(kind)
            }
            None => self.root.accepts(kind),
        };
        (!unchanged && accepted).then_some(before)
    }

    /// Where a keyboard `step` takes `source`.
    fn key_drop(&self, source: usize, step: ReorderKeyMove) -> Option<TreeDrop> {
        let carried = &Carried::Row(source);
        let at = |target, position| TreeDrop {
            target,
            position,
            anchor: target,
        };
        match step {
            ReorderKeyMove::Up => self.first_allowed(
                carried,
                [at(self.previous_sibling(source)?, TreeDropPosition::Before)],
            ),
            ReorderKeyMove::Down => self.first_allowed(
                carried,
                [at(self.next_sibling(source)?, TreeDropPosition::After)],
            ),
            ReorderKeyMove::Indent => {
                let parent = self.previous_sibling(source)?;
                let child = self.depth(parent) + 1;
                let last_child = (parent + 1..source)
                    .rev()
                    .find(|&row| self.depth(row) == child);
                self.first_allowed(
                    carried,
                    last_child
                        .map(|row| at(row, TreeDropPosition::After))
                        .into_iter()
                        .chain([at(parent, TreeDropPosition::Inside)]),
                )
            }
            ReorderKeyMove::Outdent => {
                self.first_allowed(carried, [at(self.parent(source)?, TreeDropPosition::After)])
            }
        }
    }
}

impl ReorderDrag {
    /// Follows the pointer to `(x, y)`; once it has gone [`DRAG_THRESHOLD`]
    /// from the press, the gesture has moved for good.
    fn follow(&mut self, x: f32, y: f32) {
        if !point_finite(x, y) {
            return;
        }
        self.x = x;
        self.y = y;
        let dx = x - self.start_x;
        let dy = y - self.start_y;
        self.moved |= dx * dx + dy * dy >= DRAG_THRESHOLD * DRAG_THRESHOLD;
    }
}

/// A primary press on a control inside a row, such as a button. It stays the
/// control's press, and its click, until the pointer moves past
/// [`DRAG_THRESHOLD`]; then the list takes the pointer and drags the row.
#[derive(Debug, Clone, PartialEq)]
struct ArmedPress {
    pointer_id: u64,
    /// The node the press landed on. The press is still this one while the
    /// pointer's press record names it.
    control: StableNodeId,
    drag: ReorderDrag,
}

/// One painted row. Application identities stay on [`ReorderItem`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReorderRowPaint {
    pub label: Arc<str>,
    pub selected: bool,
    pub disabled: bool,
}

/// Resolved geometry and row paints for a [`ReorderList`] component.
#[derive(Debug, Clone, PartialEq)]
pub struct ReorderListLayout {
    pub rows: Vec<(LayoutBox, crate::ComponentTextRegion, Option<[f32; 4]>)>,
    pub insert: Option<(LayoutBox, [f32; 4])>,
    pub inside: Option<(LayoutBox, [f32; 4])>,
    pub dragged: Option<(LayoutBox, [f32; 4])>,
    /// Dashed landing slot box, accent color, and whether it is nested under `inside`.
    pub slot: Option<(LayoutBox, [f32; 4], bool)>,
}

/// Visual payload projected by [`ReorderList`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReorderListVisual {
    pub rows: Arc<[ReorderRowPaint]>,
    pub size: ControlSize,
    pub spacing: f32,
    pub drag: Option<ReorderDragVisual>,
}

impl ReorderListVisual {
    fn layout_at(
        &self,
        world: &crate::UiWorld,
        id: StableNodeId,
        bounds: LayoutBox,
        palette: &nana_ui_core::SemanticPalette,
        metrics: nana_ui_core::ThemeMetrics,
    ) -> ReorderListLayout {
        // Live row children own their label painting; a stale retained `rows`
        // (projected before the children were attached) must not paint a second
        // copy underneath them. They also own the row boxes the drag feedback
        // is placed against.
        let children = world.child_ids(id);
        let live = !children.is_empty();
        let painted: &[ReorderRowPaint] = if live { &[] } else { &self.rows };
        let row_boxes = |count: usize| -> Vec<LayoutBox> {
            if live {
                children
                    .iter()
                    .filter_map(|child| world.component_layout_box(*child))
                    .collect()
            } else {
                let height = self.size.height_in(metrics);
                let step = height + self.spacing.max(0.0);
                (0..count)
                    .map(|index| LayoutBox {
                        x: bounds.x,
                        y: bounds.y + index as f32 * step,
                        width: bounds.width,
                        height,
                    })
                    .collect()
            }
        };
        let (mark, dragged, gap_slot, gap_inside) = match self.drag.as_ref() {
            Some(drag) if drag.live_gap => {
                let boxes = row_boxes(drag.rows());
                if let Some(plan) = drag.gap_plan(bounds, &boxes) {
                    (
                        None,
                        None,
                        plan.slot
                            .map(|slot| (slot, palette.accent.as_rgba_array(), plan.slot_nested)),
                        plan.inside
                            .map(|inside| (inside, palette.accent.as_rgba_array())),
                    )
                } else {
                    (None, None, None, None)
                }
            }
            Some(drag) if !drag.preview => {
                let boxes = row_boxes(drag.rows());
                (
                    drag.drop_mark(bounds, &boxes),
                    drag.carried_box(&boxes),
                    None,
                    None,
                )
            }
            _ => (None, None, None, None),
        };
        let height = self.size.height_in(metrics);
        let spacing = self.spacing.max(0.0);
        let pad = nana_ui_core::space::MD;
        let rows = painted
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let row_bounds = LayoutBox {
                    x: bounds.x,
                    y: bounds.y + index as f32 * (height + spacing),
                    width: bounds.width,
                    height,
                };
                let label = crate::ComponentTextRegion {
                    bounds: LayoutBox {
                        x: row_bounds.x + pad,
                        y: row_bounds.y,
                        width: (row_bounds.width - pad * 2.0).max(0.0),
                        height: row_bounds.height,
                    },
                    content: Arc::clone(&row.label).into(),
                    color: Some(if row.disabled {
                        palette.muted.as_rgba_array()
                    } else {
                        palette.text.as_rgba_array()
                    }),
                    font_size: self.size.text_size(),
                    font_weight: None,
                };
                let fill = row.selected.then_some(palette.selected.as_rgba_array());
                (row_bounds, label, fill)
            })
            .collect();
        ReorderListLayout {
            rows,
            insert: match mark {
                Some(ReorderDropMark::Line(line)) => Some((line, palette.accent.as_rgba_array())),
                _ => None,
            },
            inside: gap_inside.or_else(|| match mark {
                Some(ReorderDropMark::Inside(row)) => Some((row, palette.accent.as_rgba_array())),
                _ => None,
            }),
            dragged: dragged.map(|row| (row, palette.accent.as_rgba_array())),
            slot: gap_slot,
        }
    }
}

impl crate::WidgetVisual for ReorderListVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_SHAPE)
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let layout = self.layout_at(
            ctx.world,
            ctx.id,
            ctx.bounds,
            &ctx.style_model.palette,
            ctx.style_model.metrics,
        );
        let mut out = crate::WidgetPaintOutput::new();
        out.owns_node_text = true;
        out.suppress_root_surface = true;

        let selected: Vec<_> = layout
            .rows
            .iter()
            .filter_map(|(row, _, fill)| fill.map(|color| (*row, color)))
            .collect();
        if let Some(&(_, color)) = selected.first() {
            let rects: Vec<_> = selected.iter().map(|(rect, _)| *rect).collect();
            out.push(
                10,
                crate::WidgetPaintOp::QuadBatch(crate::WidgetQuadBatch {
                    bounds: crate::widget_visual::union_layout_rects(&rects),
                    rects: Arc::from(rects),
                    colors: crate::WidgetBatchColors::Uniform(color),
                    fill_paint: None,
                    border: None,
                    corner_radius: [ctx.chrome_radii.list_item; 4],
                }),
            );
        }
        // The dragged row is outlined where it was, so the row being
        // moved stays apparent while the insert line shows where it goes.
        // Both paint one level above the list, over live row children:
        // a selected row's fill would hide them otherwise.
        if let Some((row, color)) = layout.dragged {
            out.push_tagged(
                crate::WidgetTaggedOp::new(
                    12,
                    crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                        rect: row,
                        fill: None,
                        border: Some((color, 1.5)),
                        corner_radius: [ctx.chrome_radii.list_item; 4],
                        ..crate::WidgetQuad::default()
                    }),
                )
                .with_z_offset(1),
            );
        }
        // The row a drop goes into: a ring and a faint wash of the
        // accent, so its own label stays readable under it.
        if let Some((row, color)) = layout.inside {
            let [r, g, b, a] = color;
            out.push_tagged(
                crate::WidgetTaggedOp::new(
                    13,
                    crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                        rect: row,
                        fill: Some([r, g, b, a * 0.12]),
                        border: Some((color, 2.0)),
                        corner_radius: [ctx.chrome_radii.list_item; 4],
                        ..crate::WidgetQuad::default()
                    }),
                )
                .with_z_offset(1),
            );
        }
        if let Some((line, color)) = layout.insert {
            out.push_tagged(
                crate::WidgetTaggedOp::new(
                    11,
                    crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                        rect: line,
                        fill: Some(color),
                        ..crate::WidgetQuad::default()
                    }),
                )
                .with_z_offset(1),
            );
        }
        if let Some((slot_rect, color, nested)) = layout.slot {
            let [r, g, b, a] = color;
            let mut vec_bounds = slot_rect;
            let elbow_and_parent = nested.then_some(layout.inside).flatten();
            if let Some((parent_rect, _)) = elbow_and_parent {
                let min_x = vec_bounds.x.min(parent_rect.x);
                let min_y = vec_bounds.y.min(parent_rect.y + parent_rect.height);
                let max_x =
                    (vec_bounds.x + vec_bounds.width).max(parent_rect.x + parent_rect.width);
                let max_y =
                    (vec_bounds.y + vec_bounds.height).max(parent_rect.y + parent_rect.height);
                vec_bounds = LayoutBox {
                    x: min_x,
                    y: min_y,
                    width: (max_x - min_x).max(0.0),
                    height: (max_y - min_y).max(0.0),
                };
            }
            let local_slot = LayoutBox {
                x: slot_rect.x - vec_bounds.x + 0.75,
                y: slot_rect.y - vec_bounds.y + 0.75,
                width: (slot_rect.width - 1.5).max(0.0),
                height: (slot_rect.height - 1.5).max(0.0),
            };
            let mut behind = Vec::with_capacity(3);
            let mut rect_path = crate::PaintPath::new();
            rect_path.rounded_rect(local_slot, [ctx.chrome_radii.list_item; 4]);
            let rect_path = Arc::new(rect_path);
            behind.push(crate::PaintOp::FillPath {
                path: Arc::clone(&rect_path),
                paint: crate::ResolvedPaint::Solid([r, g, b, a * 0.08]),
            });
            behind.push(crate::PaintOp::StrokePath {
                path: rect_path,
                paint: crate::ResolvedPaint::Solid([r, g, b, a * 0.55]),
                stroke: crate::StrokeStyle::new(1.5)
                    .dash([5.0, 4.0], 0.0)
                    .cap(crate::LineCap::Round),
            });
            if let Some((parent_rect, _)) = elbow_and_parent {
                let elbow_x = (slot_rect.x - 10.0).max(parent_rect.x + 8.0) - vec_bounds.x;
                let start_y = parent_rect.y + parent_rect.height - vec_bounds.y;
                let mid_y = slot_rect.y + slot_rect.height * 0.5 - vec_bounds.y;
                let end_x = slot_rect.x - vec_bounds.x;
                let mut conn_path = crate::PaintPath::new();
                conn_path
                    .move_to(elbow_x, start_y)
                    .line_to(elbow_x, mid_y)
                    .line_to(end_x, mid_y);
                behind.push(crate::PaintOp::StrokePath {
                    path: Arc::new(conn_path),
                    paint: crate::ResolvedPaint::Solid([r, g, b, a * 0.6]),
                    stroke: crate::StrokeStyle::new(1.5).cap(crate::LineCap::Round),
                });
            }
            out.push_tagged(
                crate::WidgetTaggedOp::new(
                    14,
                    crate::WidgetPaintOp::Vector {
                        bounds: vec_bounds,
                        recording: Arc::new(crate::PaintRecording {
                            behind_children: behind,
                            over_children: Vec::new(),
                        }),
                    },
                )
                .with_z_offset(0),
            );
        }
        for (index, (_, label, _)) in layout.rows.iter().enumerate() {
            out.push(
                40u64.saturating_add(index as u64),
                crate::WidgetPaintOp::Text(crate::widget_visual::widget_text_region(
                    label,
                    ctx,
                    crate::TextHorizontalAlignment::Start,
                    true,
                )),
            );
        }
        out
    }
}

impl crate::UiWorld {
    /// Resolves `id`'s reorder-list row boxes and active drop indicators.
    #[must_use]
    pub fn reorder_list_layout(&self, id: StableNodeId) -> Option<ReorderListLayout> {
        let visual = self.widget_visual_as::<ReorderListVisual>(id)?;
        let bounds = self.component_layout_box(id)?;
        let style_model = self.node_style_model(id);
        Some(visual.layout_at(self, id, bounds, &style_model.palette, style_model.metrics))
    }
}

/// Vertical list. NanaUI does not mutate item order on reorder or tree drop.
#[derive(Debug, Clone, PartialEq)]
pub struct ReorderList {
    pub items: Vec<ReorderItem>,
    pub spacing: f32,
    /// Horizontal gap between tiles when [`Self::layout_mode`] is [`ReorderLayout::Grid`].
    /// Defaults to [`Self::spacing`] when `None`.
    pub column_gap: Option<f32>,
    /// Layout and drop-hit geometry mode ([`ReorderLayout::Column`] or [`ReorderLayout::Grid`]).
    pub layout_mode: ReorderLayout,
    /// How drags present landing feedback ([`ReorderDragPreview::Auto`] by default).
    pub drag_preview: ReorderDragPreview,
    pub size: ControlSize,
    pub tree_drop: bool,
    /// Rows move: a press that travels drags its row. Off, the list only
    /// selects, a click-only list of rows a drag would not change.
    pub reorder: bool,
    /// How far each level of [`ReorderItem::depth`] sits in, for the insert
    /// line and for picking a depth with the pointer.
    pub indent: f32,
    /// Which rows a tree drop may put at the top level. All by default.
    pub accepts_root: ReorderAccepts,
    /// Declared at construction: live row children own painting and the list
    /// body follows their layout. Never inferred from the tree at project time.
    pub live_rows: bool,
    /// Lists of one document that share a group take each other's drags.
    /// `None` (the default) keeps drags in the list they start in.
    pub transfer_group: Option<Arc<str>>,
    /// Outer container node (for example a collapsible section Card) whose
    /// whole bounds accept transfers and keep `inside_group = true` even over
    /// header/padding areas outside the list rows themselves.
    pub drop_zone: Option<StableNodeId>,
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
    drag: Option<ReorderDrag>,
    armed: Option<ArmedPress>,
    incoming: Option<IncomingDrag>,
}

impl ReorderList {
    pub fn new(items: impl IntoIterator<Item = ReorderItem>) -> Self {
        Self {
            items: items.into_iter().collect(),
            spacing: DEFAULT_SPACING,
            column_gap: None,
            layout_mode: ReorderLayout::Column,
            drag_preview: ReorderDragPreview::Auto,
            size: ControlSize::Small,
            tree_drop: false,
            reorder: true,
            indent: DEFAULT_INDENT,
            accepts_root: ReorderAccepts::All,
            live_rows: false,
            transfer_group: None,
            drop_zone: None,
            label: None,
            style: NodeStyle::default(),
            drag: None,
            armed: None,
            incoming: None,
        }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing.max(0.0);
        self
    }

    /// Horizontal gap between tiles when [`Self::layout_mode`] is [`ReorderLayout::Grid`].
    pub fn column_gap(mut self, gap: f32) -> Self {
        self.column_gap = Some(gap.max(0.0));
        self
    }

    /// Sets [`Self::layout_mode`](field@Self::layout_mode).
    pub fn layout_mode(mut self, mode: ReorderLayout) -> Self {
        self.layout_mode = mode;
        self
    }

    /// Sets [`Self::drag_preview`](field@Self::drag_preview).
    pub fn drag_preview(mut self, preview: ReorderDragPreview) -> Self {
        self.drag_preview = preview;
        self
    }

    /// Sets [`Self::drop_zone`](field@Self::drop_zone).
    pub fn drop_zone(mut self, zone: Option<StableNodeId>) -> Self {
        self.drop_zone = zone;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn tree_drop(mut self, enabled: bool) -> Self {
        self.tree_drop = enabled;
        self
    }

    /// See [`Self::reorder`](field@Self::reorder).
    pub fn reorder(mut self, enabled: bool) -> Self {
        self.reorder = enabled;
        self
    }

    /// See [`Self::indent`](field@Self::indent).
    pub fn indent(mut self, indent: f32) -> Self {
        self.indent = indent.max(0.0);
        self
    }

    /// See [`Self::accepts_root`](field@Self::accepts_root).
    pub fn accepts_root(mut self, accepts: ReorderAccepts) -> Self {
        self.accepts_root = accepts;
        self
    }

    /// Declare that the host attaches live row children which own painting.
    /// Retained `items` stay the drag/hit-test model and are never painted as
    /// self-drawn rows in this mode.
    pub fn live_rows(mut self, live_rows: bool) -> Self {
        self.live_rows = live_rows;
        self
    }

    /// Lets lists of one document that share `group` take each other's
    /// drags. A row dragged out of this list over another of the group lands
    /// there by that list's rules — its rows' `accepts_inside` and
    /// `accepts_sibling`, its `accepts_root` — as if dragged inside it, with
    /// the carried row's [`ReorderItem::kind`]; that list marks the drop and,
    /// on release, publishes it as its own `TreeDrop` (or `Reorder` for a
    /// flat list) whose `source` is the row's value here. This list publishes
    /// nothing for such a drop. Keyboard moves stay inside the list.
    pub fn transfer_group(mut self, group: impl Into<Arc<str>>) -> Self {
        self.transfer_group = Some(group.into());
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn selected_value(&self) -> Option<&Arc<str>> {
        self.items
            .iter()
            .find(|item| item.selected)
            .map(|item| &item.value)
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Uniform row rectangles stacked from the top of `bounds`.
    pub fn row_bounds(
        &self,
        bounds: LayoutBox,
        metrics: nana_ui_core::ThemeMetrics,
    ) -> Vec<LayoutBox> {
        let height = self.size.height_in(metrics);
        let spacing = self.spacing.max(0.0);
        self.items
            .iter()
            .enumerate()
            .map(|(index, _)| LayoutBox {
                x: bounds.x,
                y: bounds.y + index as f32 * (height + spacing),
                width: bounds.width,
                height,
            })
            .collect()
    }

    /// 4px threshold, source/target hit testing, and terminal events.
    pub fn apply_pointer(
        &mut self,
        pointer: ReorderListPointer,
        bounds: LayoutBox,
        metrics: nana_ui_core::ThemeMetrics,
    ) -> Option<ReorderListEvent> {
        let rows = self.row_bounds(bounds, metrics);
        self.apply_pointer_with_rows(pointer, &rows, &[])
    }

    /// Same as [`Self::apply_pointer`], with caller-supplied row boxes and
    /// reserved tool boxes that must not start a drag.
    pub fn apply_pointer_with_rows(
        &mut self,
        pointer: ReorderListPointer,
        rows: &[LayoutBox],
        exclude: &[LayoutBox],
    ) -> Option<ReorderListEvent> {
        match pointer {
            ReorderListPointer::Down { x, y } => self.begin(x, y, rows, exclude),
            ReorderListPointer::Move { x, y } => {
                self.move_to(x, y);
                None
            }
            ReorderListPointer::Up { x, y } => {
                self.move_to(x, y);
                self.finish(rows)
            }
            ReorderListPointer::Cancel => self.cancel(),
        }
    }

    pub fn paint_rows(&self) -> Arc<[ReorderRowPaint]> {
        self.items
            .iter()
            .map(|item| ReorderRowPaint {
                label: Arc::clone(&item.label),
                selected: item.selected,
                disabled: item.disabled,
            })
            .collect()
    }

    /// Insert-line (or inside highlight) for the active drop over the list's
    /// own uniform rows.
    pub fn insert_line(
        &self,
        bounds: LayoutBox,
        metrics: nana_ui_core::ThemeMetrics,
    ) -> Option<LayoutBox> {
        self.drag_visual()?
            .insert_line(bounds, &self.row_bounds(bounds, metrics))
    }

    /// The drag feedback to paint: present once a row has moved past the
    /// threshold, gone again when the gesture ends. While a row of another
    /// list of the [`Self::transfer_group`] is dragged over this one, the
    /// drop it would make here, without a source row.
    pub fn drag_visual(&self) -> Option<ReorderDragVisual> {
        if self.drag.is_none() {
            return self.incoming_visual();
        }
        let drag = self.drag.as_ref().filter(|drag| drag.moved)?;
        let source = self.item_index(&drag.source)?;
        if !self.items[source].is_source() {
            return None;
        }
        let live_gap = self.live_rows && self.drag_preview == ReorderDragPreview::LiveGap;
        let preview = match self.drag_preview {
            ReorderDragPreview::Auto => self.live_rows && !self.tree_drop,
            ReorderDragPreview::LiveGap => self.live_rows,
            ReorderDragPreview::Marks => false,
        };
        Some(ReorderDragVisual {
            source: Some(source),
            x: drag.x,
            y: drag.y,
            press_x: drag.start_x,
            press_y: drag.start_y,
            tree_drop: self.tree_drop,
            preview,
            live_gap,
            layout_mode: self.layout_mode,
            rules: Arc::new(DropRules::of(self)),
            carried: Carried::Row(source),
            elsewhere: drag.over.is_some(),
            outside_group: drag.outside_group,
            origin: drag.origin,
            carried_size: None,
            carried_step: None,
        })
    }

    fn incoming_visual(&self) -> Option<ReorderDragVisual> {
        let incoming = self.incoming.as_ref()?;
        let live_gap = self.live_rows && self.drag_preview == ReorderDragPreview::LiveGap;
        Some(ReorderDragVisual {
            source: None,
            x: incoming.x,
            y: incoming.y,
            press_x: incoming.x,
            press_y: incoming.y,
            tree_drop: self.tree_drop,
            // In Auto/Marks modes, nothing of this list follows the pointer: the drop is marked.
            // In LiveGap mode, presentation shifting opens room in the target list.
            preview: live_gap,
            live_gap,
            layout_mode: self.layout_mode,
            rules: Arc::new(DropRules::of(self)),
            carried: Carried::Foreign(incoming.kind.clone()),
            elsewhere: false,
            outside_group: false,
            origin: incoming.origin,
            carried_size: incoming.carried_size,
            carried_step: incoming.carried_step,
        })
    }

    /// What this list's drag carries out to another list of its transfer
    /// group: the group, the dragged row's value and kind. `None` without a
    /// group or before the drag has moved.
    fn transfer_carry(&self) -> Option<(Arc<str>, Arc<str>, Option<Arc<str>>)> {
        let group = self.transfer_group.clone()?;
        let drag = self.drag.as_ref().filter(|drag| drag.moved)?;
        let item = &self.items[self.item_index(&drag.source)?];
        item.is_source()
            .then(|| (group, Arc::clone(&item.value), item.kind.clone()))
    }

    /// Whether this list accepts an external drag of `kind`.
    fn takes_external(&self, kind: Option<&str>) -> bool {
        self.drag.is_none()
            && (self.accepts_root.accepts(kind)
                || self
                    .items
                    .iter()
                    .any(|item| item.is_drop_target() && item.accepts_sibling.accepts(kind)))
    }

    /// Whether this list takes a drag of another list in `group` now.
    fn takes_transfer(&self, group: &str) -> bool {
        self.transfer_group.as_deref() == Some(group)
            && self.drag.is_none()
            && (self.accepts_root != ReorderAccepts::Nothing
                || self.items.iter().any(ReorderItem::is_drop_target))
    }

    /// The event a drag dragged in from another list publishes here on
    /// release over `rows`; clears it either way.
    fn finish_incoming(&mut self, rows: &[LayoutBox]) -> Option<ReorderListEvent> {
        let incoming = self.incoming.take()?;
        let rules = DropRules::of(self);
        let carried = Carried::Foreign(incoming.kind);
        if self.tree_drop {
            let drop = rules.tree_drop(rows, &carried, incoming.x, incoming.y)?;
            return Some(ReorderListEvent::TreeDrop {
                source: incoming.value,
                intent: TreeDropIntent {
                    target: Arc::clone(&self.items[drop.target].value),
                    position: drop.position,
                },
            });
        }
        let before = if self.layout_mode == ReorderLayout::Grid {
            rules.grid_drop(rows, &carried, incoming.x, incoming.y)?
        } else {
            rules.flat_drop(rows, &carried, incoming.y)?
        };
        Some(ReorderListEvent::Reorder {
            source: incoming.value,
            before: before.map(|index| Arc::clone(&self.items[index].value)),
        })
    }

    /// The event a keyboard `step` of row `source` publishes: a
    /// [`ReorderListEvent::TreeDrop`] with [`Self::tree_drop`], else a
    /// [`ReorderListEvent::Reorder`] (only [`ReorderKeyMove::Up`] and
    /// [`ReorderKeyMove::Down`]). `None` where the row cannot go, or would
    /// not move. The row carries the rows it holds.
    pub fn key_move(&self, source: &str, step: ReorderKeyMove) -> Option<ReorderListEvent> {
        let index = self.item_index(source)?;
        if !self.moves(source) || self.drag.is_some() || !self.items[index].is_source() {
            return None;
        }
        let rules = DropRules::of(self);
        let source = Arc::clone(&self.items[index].value);
        if self.tree_drop {
            let drop = rules.key_drop(index, step)?;
            return Some(ReorderListEvent::TreeDrop {
                source,
                intent: TreeDropIntent {
                    target: Arc::clone(&self.items[drop.target].value),
                    position: drop.position,
                },
            });
        }
        let before = match step {
            ReorderKeyMove::Up => Some(rules.previous_sibling(index)?),
            ReorderKeyMove::Down => {
                let after = rules.subtree_end(rules.next_sibling(index)?) + 1;
                (after < self.items.len()).then_some(after)
            }
            ReorderKeyMove::Indent | ReorderKeyMove::Outdent => return None,
        };
        let kind = self.items[index].kind.as_deref();
        if let Some(before) = before
            && !(rules.rows[before].target && rules.rows[before].sibling.accepts(kind))
        {
            return None;
        }
        Some(ReorderListEvent::Reorder {
            source,
            before: before.map(|before| Arc::clone(&self.items[before].value)),
        })
    }

    /// Clears an in-flight gesture. Escape, unfocus, and lost-touch use this.
    /// A press on a row's control that has not moved yet was never a drag:
    /// it is dropped without [`ReorderListEvent::Cancelled`].
    pub fn cancel(&mut self) -> Option<ReorderListEvent> {
        self.armed = None;
        self.incoming = None;
        self.drag.take().map(|_| ReorderListEvent::Cancelled)
    }

    fn begin(
        &mut self,
        x: f32,
        y: f32,
        rows: &[LayoutBox],
        exclude: &[LayoutBox],
    ) -> Option<ReorderListEvent> {
        if self.drag.is_some() || !point_finite(x, y) {
            return None;
        }
        // A press here ends whatever another list left showing.
        self.incoming = None;
        if exclude.iter().any(|reserved| reserved.contains(x, y)) {
            return None;
        }
        let sources = self
            .items
            .iter()
            .map(ReorderItem::is_source)
            .collect::<Vec<_>>();
        let index = item_at(rows, &sources, x, y)?;
        self.drag = Some(ReorderDrag {
            source: Arc::clone(&self.items[index].value),
            start_x: x,
            start_y: y,
            x,
            y,
            moved: false,
            over: None,
            outside_group: false,
            drag_emitted: false,
            origin: None,
        });
        None
    }

    fn move_to(&mut self, x: f32, y: f32) {
        // never moved, the release is a click
        if !self
            .drag
            .as_ref()
            .is_some_and(|drag| self.moves(&drag.source))
        {
            return;
        }
        if let Some(drag) = self.drag.as_mut() {
            drag.follow(x, y);
        }
    }

    fn finish(&mut self, rows: &[LayoutBox]) -> Option<ReorderListEvent> {
        let drag = self.drag.take()?;
        let source = self.item_index(&drag.source)?;
        if !self.items[source].is_source() {
            return None;
        }
        if !drag.moved {
            self.set_selected(drag.source.as_ref());
            return Some(ReorderListEvent::Select(drag.source));
        }
        if drag.outside_group {
            return None;
        }
        let rules = DropRules::of(self);
        let carried = Carried::Row(source);
        if self.tree_drop {
            let drop = rules.tree_drop(rows, &carried, drag.x, drag.y)?;
            let target = Arc::clone(&self.items[drop.target].value);
            return Some(ReorderListEvent::TreeDrop {
                source: drag.source,
                intent: TreeDropIntent {
                    target,
                    position: drop.position,
                },
            });
        }
        let before = if self.layout_mode == ReorderLayout::Grid {
            rules.grid_drop(rows, &carried, drag.x, drag.y)?
        } else {
            rules.flat_drop(rows, &carried, drag.y)?
        };
        let before = before.map(|index| Arc::clone(&self.items[index].value));
        Some(ReorderListEvent::Reorder {
            source: drag.source,
            before,
        })
    }

    /// Whether row `value` moves when dragged.
    fn moves(&self, value: &str) -> bool {
        self.reorder
            && self
                .item_index(value)
                .is_some_and(|index| self.items[index].movable)
    }

    fn item_index(&self, value: &str) -> Option<usize> {
        self.items
            .iter()
            .position(|item| item.value.as_ref() == value)
    }

    fn set_selected(&mut self, value: &str) {
        for item in &mut self.items {
            item.selected = item.value.as_ref() == value && !item.disabled;
        }
    }

    fn selected_label(&self) -> Option<Arc<str>> {
        self.items
            .iter()
            .find(|item| item.selected)
            .map(|item| Arc::clone(&item.label))
    }

    fn intrinsic_height(&self, metrics: nana_ui_core::ThemeMetrics) -> f32 {
        let count = self.items.len().max(1) as f32;
        count * self.size.height_in(metrics) + (count - 1.0) * self.spacing.max(0.0)
    }
}

impl Default for ReorderList {
    fn default() -> Self {
        Self::new([])
    }
}

impl ComponentView for ReorderList {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::REORDER_LIST.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "reorder-list".into(),
        }
    }

    fn wants_child_reproject() -> bool {
        true
    }

    fn wants_metrics_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        // Live row children paint themselves; retained items only feed
        // hit testing and reorder events.
        let live_rows = self.live_rows;
        let visual = ReorderListVisual {
            rows: if live_rows {
                Arc::<[ReorderRowPaint]>::from([])
            } else {
                self.paint_rows()
            },
            size: self.size,
            spacing: self.spacing,
            drag: self.drag_visual(),
        };
        if world.widget_visual_as::<ReorderListVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        if layout.width.is_none() {
            layout.width = Some(LengthSpec::Fill);
        }
        if live_rows {
            if self.layout_mode == ReorderLayout::Grid {
                layout.direction = Some(FlexDirection::Row);
                layout.flex_wrap = FlexWrap::Wrap;
                if layout.row_gap.is_none() {
                    layout.row_gap = Some(LengthSpec::Px(self.spacing.max(0.0)));
                }
                if layout.column_gap.is_none() {
                    layout.column_gap = Some(LengthSpec::Px(
                        self.column_gap.unwrap_or(self.spacing).max(0.0),
                    ));
                }
            } else {
                layout.direction = Some(FlexDirection::Column);
                if layout.gap.is_none() {
                    layout.gap = Some(LengthSpec::Px(self.spacing.max(0.0)));
                }
            }
            if layout.height.is_none() {
                layout.height = Some(LengthSpec::Shrink);
            }
            if self.drag_preview == ReorderDragPreview::LiveGap
                && let Some(incoming) = &self.incoming
                && self.accepts_root.accepts(incoming.kind.as_deref())
            {
                let extra = incoming.carried_step.unwrap_or_else(|| {
                    incoming
                        .carried_size
                        .map_or(self.size.height_in(world.theme_metrics()), |s| s[1])
                        + self.spacing.max(0.0)
                });
                let needs_room = if self.layout_mode == ReorderLayout::Grid {
                    let children = world.child_ids(id);
                    if children.is_empty() {
                        true
                    } else if let (Some(bounds), Some(last)) = (
                        world.component_layout_box(id),
                        children.last().and_then(|c| world.component_layout_box(*c)),
                    ) {
                        let col_gap = self.column_gap.unwrap_or(self.spacing).max(0.0);
                        let item_w = incoming.carried_size.map_or(last.width, |s| s[0]);
                        last.x - bounds.x + last.width + col_gap + item_w > bounds.width + 0.5
                    } else {
                        false
                    }
                } else {
                    true
                };
                if needs_room && extra > 0.0 {
                    let bottom = layout.resolved_padding().bottom + extra;
                    layout.padding_bottom = Some(LengthSpec::Px(bottom));
                }
            }
        } else {
            // Border-box: declared padding is added to the self-drawn row
            // stack instead of clipping its last row.
            let padding = layout.resolved_padding();
            let content =
                self.intrinsic_height(world.theme_metrics()) + padding.top + padding.bottom;
            if layout.height.is_none() {
                layout.height = Some(LengthSpec::Px(content));
            }
            if layout.min_height.is_none() {
                layout.min_height = Some(LengthSpec::Px(content));
            }
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::List,
                label: self.label.clone().or_else(|| self.selected_label()),
                value: self.selected_value().cloned(),
                disabled: false,
                ..AccessibilityState::default()
            },
        );
    }
}

impl crate::AppContext {
    pub fn is_reorder_list(&self, id: StableNodeId) -> bool {
        self.read(crate::Entity::<ReorderList>::from_stable_id(id), |_| ())
            .is_ok()
    }

    pub fn nearest_reorder_list(&self, mut id: StableNodeId) -> Option<StableNodeId> {
        loop {
            if self.is_reorder_list(id) {
                return Some(id);
            }
            id = self.world().parent_id(id)?;
        }
    }

    /// Alt with an arrow key on a focused list, or on a focused control in
    /// one of its rows: moves that row, else the chosen one, by
    /// [`ReorderList::key_move`] — up or down past a sibling, right into the
    /// sibling above, left out of its parent. Publishes the move as a drop
    /// would; returns whether one was published.
    pub fn focused_reorder_list_key(
        &mut self,
        document: crate::DocumentId,
        key: &str,
    ) -> Result<bool, crate::FrameworkError> {
        let step = match key {
            "ArrowUp" => ReorderKeyMove::Up,
            "ArrowDown" => ReorderKeyMove::Down,
            "ArrowRight" => ReorderKeyMove::Indent,
            "ArrowLeft" => ReorderKeyMove::Outdent,
            _ => return Ok(false),
        };
        let Some(focused) = self.world().focused(document) else {
            return Ok(false);
        };
        let Some(list_id) = self.nearest_reorder_list(focused) else {
            return Ok(false);
        };
        let Some(entity) = self.reorder_list_entity(list_id) else {
            return Ok(false);
        };
        // The live row the focus is in: the list's child on its way up.
        let mut row = focused;
        while row != list_id && self.world().parent_id(row) != Some(list_id) {
            let Some(parent) = self.world().parent_id(row) else {
                break;
            };
            row = parent;
        }
        let row = (row != list_id)
            .then(|| {
                self.world()
                    .node(list_id)
                    .and_then(|node| node.children.iter().position(|child| *child == row))
            })
            .flatten();
        self.update_component(entity, |list, cx| {
            let source = row
                .and_then(|row| list.items.get(row))
                .map(|item| Arc::clone(&item.value))
                .or_else(|| list.selected_value().cloned());
            let Some(event) = source.and_then(|source| list.key_move(&source, step)) else {
                return false;
            };
            cx.emit(event);
            true
        })
    }

    /// A primary press on `target`. On a row's own surface the list takes
    /// the pointer at once: the press is the row's, a click selects it and a
    /// move drags it. On a control inside a row (a focusable node below the
    /// row, such as a button) the press stays the control's and returns
    /// `Ok(false)`, so the control is pressed and clicked as usual; the list
    /// only arms a drag that [`Self::update_reorder_list_pointer`] starts
    /// once the pointer moves past the drag threshold.
    pub fn begin_reorder_list_pointer(
        &mut self,
        document: crate::DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(list_id) = self.nearest_reorder_list(target) else {
            return Ok(false);
        };
        let Some(entity) = self.reorder_list_entity(list_id) else {
            return Ok(false);
        };
        let Some(bounds) = self.world().component_layout_box(list_id) else {
            return Ok(false);
        };
        // Row boxes are layout coordinates; an ancestor's scroll or
        // transform moves the window point away from them.
        let Some((x, y)) = self.world().pointer_layout_position(list_id, x, y) else {
            return Ok(false);
        };
        let rows = self.reorder_row_boxes(list_id, bounds);
        let exclude = self.reorder_tool_boxes(entity);
        if exclude.iter().any(|reserved| reserved.contains(x, y)) {
            return Ok(false);
        }
        let on_control = self.is_reorder_row_control(list_id, target);
        self.update_component(entity, |list, cx| {
            // A new press replaces one armed before it whose release went to
            // whatever its control started (a slider's drag, say).
            list.armed = None;
            let in_flight = list.is_dragging();
            list.apply_pointer_with_rows(ReorderListPointer::Down { x, y }, &rows, &exclude);
            if !list.is_dragging() {
                return false;
            }
            if !in_flight && let Some(drag) = list.drag.as_mut() {
                drag.origin = Some([bounds.x, bounds.y]);
            }
            if on_control && !in_flight {
                list.armed = list.drag.take().map(|drag| ArmedPress {
                    pointer_id,
                    control: target,
                    drag,
                });
                return false;
            }
            cx.mutations().capture_pointer(pointer_id, list_id);
            cx.mutations().request_focus(document, Some(list_id));
            true
        })
    }

    /// Moves the drag that holds `pointer_id`. Over another list of its
    /// [`ReorderList::transfer_group`], that list shows where the row would
    /// land instead of this one.
    pub fn update_reorder_list_pointer(
        &mut self,
        document: crate::DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            return self.follow_armed_reorder_press(document, pointer_id, x, y);
        };
        let Some(entity) = self.reorder_list_entity(target) else {
            return Ok(false);
        };
        let Some(bounds) = self.world().component_layout_box(target) else {
            return Ok(false);
        };
        let Some((layout_x, layout_y)) = self.world().pointer_layout_position(target, x, y) else {
            return Ok(false);
        };
        let rows = self.reorder_row_boxes(target, bounds);
        let dragging = self.update_component(entity, |list, _| {
            let pointer = ReorderListPointer::Move {
                x: layout_x,
                y: layout_y,
            };
            list.apply_pointer_with_rows(pointer, &rows, &[]);
            if let Some(drag) = list.drag.as_mut() {
                drag.origin = Some([bounds.x, bounds.y]);
            }
            list.is_dragging()
        })?;
        if dragging {
            self.route_reorder_transfer(document, target, x, y)?;
            self.update_live_gap_drag_state(target, layout_x, layout_y, x, y)?;
        }
        Ok(dragging)
    }

    /// Drives incoming live-gap preview on `list_id` from an external drag
    /// (for example a cross-window drag or a non-`ReorderList` drawer). Passing
    /// `None` or coordinates outside `list_id`'s bounds / [`ReorderList::drop_zone`]
    /// clears any active external preview. Returns whether the preview state changed.
    pub fn set_reorder_external_drag(
        &mut self,
        list_id: StableNodeId,
        drag: Option<ReorderExternalDrag>,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(entity) = self.reorder_list_entity(list_id) else {
            return Ok(false);
        };
        let Some(ext) = drag else {
            return self.update_component(entity, |list, _| list.incoming.take().is_some());
        };
        let world = self.world();
        if self.is_reorder_node_hidden(world, list_id) {
            return self.update_component(entity, |list, _| list.incoming.take().is_some());
        }
        let zone = self
            .read(entity, |list| list.drop_zone)
            .ok()
            .flatten()
            .unwrap_or(list_id);
        let inside_zone = world
            .pointer_layout_position(zone, ext.x, ext.y)
            .zip(world.component_layout_box(zone))
            .is_some_and(|((zx, zy), zbox)| zbox.contains(zx, zy));
        if !inside_zone {
            return self.update_component(entity, |list, _| list.incoming.take().is_some());
        }
        let Some(bounds) = world.component_layout_box(list_id) else {
            return self.update_component(entity, |list, _| list.incoming.take().is_some());
        };
        let Some((lx, ly)) = world.pointer_layout_position(list_id, ext.x, ext.y) else {
            return self.update_component(entity, |list, _| list.incoming.take().is_some());
        };
        let incoming = IncomingDrag {
            value: ext.value,
            kind: ext.kind,
            x: lx,
            y: ly,
            origin: Some([bounds.x, bounds.y]),
            carried_size: ext.size,
            carried_step: None,
        };
        self.update_component(entity, |list, _| {
            if !list.takes_external(incoming.kind.as_deref()) {
                return list.incoming.take().is_some();
            }
            let step = incoming.carried_size.map(|s| s[1] + list.spacing.max(0.0));
            let next = IncomingDrag {
                carried_step: step,
                ..incoming
            };
            let changed = list.incoming.as_ref() != Some(&next);
            list.incoming = Some(next);
            changed
        })
    }

    /// Commits an active external drag on `list_id` (clearing its incoming state)
    /// and returns the resulting [`ReorderListEvent`] without emitting on the component.
    pub fn finish_reorder_external_drag(
        &mut self,
        list_id: StableNodeId,
    ) -> Result<Option<ReorderListEvent>, crate::FrameworkError> {
        let Some(entity) = self.reorder_list_entity(list_id) else {
            return Ok(None);
        };
        let bounds = self.world().component_layout_box(list_id);
        let rows = bounds
            .map(|bounds| self.reorder_row_boxes(list_id, bounds))
            .unwrap_or_default();
        self.update_component(entity, |list, _| list.finish_incoming(&rows))
    }

    pub fn end_reorder_list_pointer(
        &mut self,
        document: crate::DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        cancel: bool,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(target) = self.world().pointer_capture(document, pointer_id) else {
            // A press that never moved far enough ends as its control's
            // click (or cancel): the release goes on to the control.
            self.disarm_reorder_press(document, pointer_id)?;
            return Ok(false);
        };
        let Some(entity) = self.reorder_list_entity(target) else {
            return Ok(false);
        };
        // A point that no longer maps into the list cancels; either way the
        // capture is released.
        let point = self.world().pointer_layout_position(target, x, y);
        let cancel = cancel || point.is_none();
        // A release over another list of the transfer group is that list's
        // drop: it publishes it, and this list publishes nothing.
        if !cancel {
            self.route_reorder_transfer(document, target, x, y)?;
            if let Some((lx, ly)) = point {
                let (over, zone, has_group, moved, live_gap) = self
                    .read(entity, |list| {
                        (
                            list.drag.as_ref().and_then(|d| d.over),
                            list.drop_zone,
                            list.transfer_group.is_some()
                                || list.layout_mode == ReorderLayout::Grid,
                            list.drag.as_ref().is_some_and(|d| d.moved),
                            list.live_rows && list.drag_preview == ReorderDragPreview::LiveGap,
                        )
                    })
                    .unwrap_or((None, None, false, false, false));
                if moved && live_gap {
                    let inside = over.is_some()
                        || self.is_inside_own_zone(target, zone, has_group, lx, ly, x, y);
                    self.update_component(entity, |list, _| {
                        if let Some(drag) = list.drag.as_mut() {
                            drag.outside_group = !inside;
                        }
                    })?;
                }
            }
        }
        let over = self
            .read(entity, |list| list.drag.as_ref().and_then(|drag| drag.over))
            .ok()
            .flatten();
        if let Some(other) = over.and_then(|other| self.reorder_list_entity(other)) {
            let bounds = self.world().component_layout_box(other.stable_id());
            let rows = bounds
                .map(|bounds| self.reorder_row_boxes(other.stable_id(), bounds))
                .unwrap_or_default();
            self.update_component(other, |list, cx| {
                let event = list.finish_incoming(&rows);
                if let Some(event) = event.filter(|_| !cancel) {
                    cx.emit(event);
                }
            })?;
            if !cancel {
                return self.update_component(entity, |list, cx| {
                    if let Some(drag) = list.drag.take()
                        && drag.drag_emitted
                    {
                        cx.emit(ReorderListEvent::Drag {
                            source: drag.source,
                            phase: PointerDragPhase::Ended,
                            x,
                            y,
                            inside_group: true,
                        });
                    }
                    list.armed = None;
                    cx.mutations().release_pointer(pointer_id, target);
                    true
                });
            }
        }
        let (lx, ly) = point.unwrap_or((x, y));
        let bounds = self.world().component_layout_box(target);
        let rows = bounds
            .map(|bounds| self.reorder_row_boxes(target, bounds))
            .unwrap_or_default();
        self.update_component(entity, |list, cx| {
            let drag_meta = list.drag.as_ref().map(|drag| {
                (
                    Arc::clone(&drag.source),
                    drag.drag_emitted,
                    drag.outside_group,
                )
            });
            let pointer = if cancel {
                ReorderListPointer::Cancel
            } else {
                ReorderListPointer::Up { x: lx, y: ly }
            };
            if let Some(event) = list.apply_pointer_with_rows(pointer, &rows, &[]) {
                cx.emit(event);
            }
            if let Some((source, drag_emitted, outside_group)) = drag_meta
                && drag_emitted
            {
                let (phase, inside_group) = if cancel {
                    (PointerDragPhase::Cancelled, false)
                } else {
                    (PointerDragPhase::Ended, !outside_group)
                };
                cx.emit(ReorderListEvent::Drag {
                    source,
                    phase,
                    x,
                    y,
                    inside_group,
                });
            }
            cx.mutations().release_pointer(pointer_id, target);
            true
        })
    }

    /// Escape cancels a list drag that holds the pointer in `document`. The
    /// press record goes too, so the release that follows neither drops the
    /// row nor clicks the control the drag started on.
    pub fn cancel_reorder_list_drag(
        &mut self,
        document: crate::DocumentId,
    ) -> Result<bool, crate::FrameworkError> {
        let mut cancelled = false;
        for (pointer_id, target) in self.world().pointer_captures(document) {
            if self.is_reorder_list(target)
                && self.end_reorder_list_pointer(document, pointer_id, 0.0, 0.0, true)?
            {
                self.release_pointer(document, pointer_id);
                cancelled = true;
            }
        }
        Ok(cancelled)
    }

    fn is_inside_own_zone(
        &self,
        list_id: StableNodeId,
        zone: Option<StableNodeId>,
        has_group: bool,
        layout_x: f32,
        layout_y: f32,
        win_x: f32,
        win_y: f32,
    ) -> bool {
        let world = self.world();
        if let Some(zone) = zone {
            return world
                .pointer_layout_position(zone, win_x, win_y)
                .zip(world.component_layout_box(zone))
                .is_some_and(|((zx, zy), zbox)| zbox.contains(zx, zy));
        }
        if has_group {
            return world
                .component_layout_box(list_id)
                .is_some_and(|bounds| bounds.contains(layout_x, layout_y));
        }
        true
    }

    fn update_live_gap_drag_state(
        &mut self,
        list_id: StableNodeId,
        layout_x: f32,
        layout_y: f32,
        win_x: f32,
        win_y: f32,
    ) -> Result<(), crate::FrameworkError> {
        let Some(entity) = self.reorder_list_entity(list_id) else {
            return Ok(());
        };
        let (live_gap, has_group, over, zone, moved) = self
            .read(entity, |list| {
                (
                    list.live_rows && list.drag_preview == ReorderDragPreview::LiveGap,
                    list.transfer_group.is_some() || list.layout_mode == ReorderLayout::Grid,
                    list.drag.as_ref().and_then(|d| d.over),
                    list.drop_zone,
                    list.drag.as_ref().is_some_and(|d| d.moved),
                )
            })
            .unwrap_or((false, false, None, None, false));
        if !moved || !live_gap {
            return Ok(());
        }
        let inside_group = over.is_some()
            || self.is_inside_own_zone(list_id, zone, has_group, layout_x, layout_y, win_x, win_y);
        self.update_component(entity, |list, cx| {
            let Some(drag) = list.drag.as_mut() else {
                return;
            };
            drag.outside_group = !inside_group;
            let phase = if drag.drag_emitted {
                PointerDragPhase::Moved
            } else {
                drag.drag_emitted = true;
                PointerDragPhase::Started
            };
            cx.emit(ReorderListEvent::Drag {
                source: Arc::clone(&drag.source),
                phase,
                x: win_x,
                y: win_y,
                inside_group,
            });
        })?;
        Ok(())
    }

    /// Points the drag list `source` holds at the list of its transfer group
    /// under window point `(x, y)`, if any: that list shows the drop and
    /// `source` stops showing its own. A list the pointer has left forgets it.
    fn route_reorder_transfer(
        &mut self,
        document: crate::DocumentId,
        source: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<(), crate::FrameworkError> {
        let Some(entity) = self.reorder_list_entity(source) else {
            return Ok(());
        };
        let (carry, previous) = self
            .read(entity, |list| {
                (
                    list.transfer_carry(),
                    list.drag.as_ref().and_then(|drag| drag.over),
                )
            })
            .unwrap_or((None, None));
        let carried_metrics = carry.as_ref().and_then(|_| {
            let src_bounds = self.world().component_layout_box(source)?;
            let src_rows = self.reorder_row_boxes(source, src_bounds);
            let (carried, spacing) = self
                .read(entity, |list| {
                    (list.drag_visual().and_then(|v| v.carried()), list.spacing)
                })
                .ok()?;
            let carried = carried?;
            let first = src_rows.get(*carried.start())?;
            let last = src_rows.get(*carried.end()).unwrap_or(first);
            let left = first.x.min(last.x);
            let right = (first.x + first.width).max(last.x + last.width);
            let size = [right - left, last.y + last.height - first.y];
            let step = carried_block_step(&src_rows, *carried.start(), *carried.end(), spacing);
            Some((size, step))
        });
        let over = carry.and_then(|(group, value, kind)| {
            let (list, target_x, target_y) =
                self.reorder_transfer_target(document, source, &group, x, y)?;
            let origin = self
                .world()
                .component_layout_box(list.stable_id())
                .map(|b| [b.x, b.y]);
            Some((
                list,
                IncomingDrag {
                    value,
                    kind,
                    x: target_x,
                    y: target_y,
                    origin,
                    carried_size: carried_metrics.map(|m| m.0),
                    carried_step: carried_metrics.map(|m| m.1),
                },
            ))
        });
        let next = over.as_ref().map(|(list, _)| list.stable_id());
        if previous != next
            && let Some(previous) = previous.and_then(|id| self.reorder_list_entity(id))
        {
            self.update_component(previous, |list, _| list.incoming = None)?;
        }
        if let Some((list, incoming)) = over {
            self.update_component(list, |list, _| list.incoming = Some(incoming))?;
        }
        if previous != next {
            self.update_component(entity, |list, _| {
                if let Some(drag) = list.drag.as_mut() {
                    drag.over = next;
                }
            })?;
        }
        Ok(())
    }

    /// The list of `group` other than `source` that is topmost under window
    /// point `(x, y)`, with the point in its layout coordinates. Over
    /// `source` itself — the rows it carries along aside — there is none:
    /// the drag stays its own.
    fn is_reorder_node_hidden(&self, world: &UiWorld, mut node: StableNodeId) -> bool {
        loop {
            if world.node_style(node).is_some_and(|s| s.layout.hidden) {
                return true;
            }
            match world.parent_id(node) {
                Some(parent) => node = parent,
                None => return false,
            }
        }
    }

    fn reorder_transfer_target(
        &self,
        document: crate::DocumentId,
        source: StableNodeId,
        group: &str,
        x: f32,
        y: f32,
    ) -> Option<(crate::Entity<ReorderList>, f32, f32)> {
        let world = self.world();
        let within = |mut node: StableNodeId, ancestor: StableNodeId| loop {
            if node == ancestor {
                return true;
            }
            match world.parent_id(node) {
                Some(parent) => node = parent,
                None => return false,
            }
        };
        // A live, flat drag's carried rows follow the pointer: whatever lies
        // under them decides.
        let carried = self
            .read(
                crate::Entity::<ReorderList>::from_stable_id(source),
                |list| list.drag_visual().and_then(|visual| visual.carried()),
            )
            .ok()
            .flatten();
        let carried_rows = world
            .node(source)
            .map(|node| {
                node.children
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| carried.as_ref().is_some_and(|c| c.contains(index)))
                    .map(|(_, child)| *child)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let hit = world
            .hit_test_candidates(document, x, y)
            .into_iter()
            .find(|hit| !carried_rows.iter().any(|row| within(*hit, *row)));
        if let Some(hit) = hit {
            if within(hit, source) {
                return None;
            }
            if let Some(list_id) = self.nearest_reorder_list(hit)
                && !self.is_reorder_node_hidden(world, list_id)
                && let Some(list) = self.reorder_list_entity(list_id)
                && self.read(list, |list| list.takes_transfer(group)).ok()?
            {
                let (lx, ly) = world.pointer_layout_position(list.stable_id(), x, y)?;
                return Some((list, lx, ly));
            }
        }
        let mut best: Option<(f32, crate::Entity<ReorderList>, f32, f32)> = None;
        for other_id in
            world.nodes_of_component(document, crate::component_descriptors::REORDER_LIST.type_id)
        {
            if other_id == source || self.is_reorder_node_hidden(world, other_id) {
                continue;
            }
            let Some(other) = self.reorder_list_entity(other_id) else {
                continue;
            };
            let Ok((takes, zone)) =
                self.read(other, |list| (list.takes_transfer(group), list.drop_zone))
            else {
                continue;
            };
            let Some(zone) = zone.filter(|_| takes) else {
                continue;
            };
            if within(source, zone) || hit.is_some_and(|h| !(within(h, zone) || within(zone, h))) {
                continue;
            }
            let Some(((zx, zy), zbox)) = world
                .pointer_layout_position(zone, x, y)
                .zip(world.component_layout_box(zone))
            else {
                continue;
            };
            if !zbox.contains(zx, zy) {
                continue;
            }
            let Some((lx, ly)) = world.pointer_layout_position(other_id, x, y) else {
                continue;
            };
            let dist = world.component_layout_box(other_id).map_or(f32::MAX, |b| {
                if ly < b.y {
                    b.y - ly
                } else if ly > b.y + b.height {
                    ly - (b.y + b.height)
                } else {
                    0.0
                }
            });
            if best.as_ref().is_none_or(|(d, _, _, _)| dist < *d) {
                best = Some((dist, other, lx, ly));
            }
        }
        best.map(|(_, list, lx, ly)| (list, lx, ly))
    }

    fn reorder_list_entity(&self, id: StableNodeId) -> Option<crate::Entity<ReorderList>> {
        self.is_reorder_list(id)
            .then(|| crate::Entity::from_stable_id(id))
    }

    /// Whether a press on `target` lands on a control inside one of `list`'s
    /// rows: a focusable node between `target` and the row, the row itself
    /// excluded. The list and a row's own surface are the row's to press.
    fn is_reorder_row_control(&self, list: StableNodeId, target: StableNodeId) -> bool {
        let world = self.world();
        let mut node = target;
        while node != list {
            let Some(parent) = world.parent_id(node) else {
                return false;
            };
            if parent == list {
                return false;
            }
            if world
                .interaction(node)
                .is_some_and(|interaction| interaction.focusable)
            {
                return true;
            }
            node = parent;
        }
        false
    }

    /// Follows a press armed on a row's control. Past the drag threshold the
    /// list takes the pointer and the control's press ends without a click.
    fn follow_armed_reorder_press(
        &mut self,
        document: crate::DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, crate::FrameworkError> {
        let Some(entity) = self.armed_reorder_press(document, pointer_id) else {
            return Ok(false);
        };
        let list_id = entity.stable_id();
        let Some(bounds) = self.world().component_layout_box(list_id) else {
            return Ok(false);
        };
        let Some((lx, ly)) = self.world().pointer_layout_position(list_id, x, y) else {
            return Ok(false);
        };
        let started = self.update_component(entity, |list, cx| {
            if !list
                .armed
                .as_ref()
                .is_some_and(|armed| list.moves(&armed.drag.source))
            {
                return false;
            }
            let Some(armed) = list.armed.as_mut() else {
                return false;
            };
            armed.drag.follow(lx, ly);
            armed.drag.origin = Some([bounds.x, bounds.y]);
            if !armed.drag.moved {
                return false;
            }
            list.drag = list.armed.take().map(|armed| armed.drag);
            cx.mutations().capture_pointer(pointer_id, list_id);
            cx.mutations().request_focus(document, Some(list_id));
            true
        })?;
        if started {
            self.release_pointer(document, pointer_id);
            self.route_reorder_transfer(document, list_id, x, y)?;
            self.update_live_gap_drag_state(list_id, lx, ly, x, y)?;
        }
        Ok(started)
    }

    /// Drops the press armed on a row's control when its pointer is released
    /// or cancelled before it moved far enough to drag.
    fn disarm_reorder_press(
        &mut self,
        document: crate::DocumentId,
        pointer_id: u64,
    ) -> Result<(), crate::FrameworkError> {
        if let Some(entity) = self.armed_reorder_press(document, pointer_id) {
            self.update_component(entity, |list, _| list.armed = None)?;
        }
        Ok(())
    }

    /// The list holding `pointer_id`'s press armed: the pointer is pressed on
    /// a node of a list whose armed press is that very press.
    fn armed_reorder_press(
        &self,
        document: crate::DocumentId,
        pointer_id: u64,
    ) -> Option<crate::Entity<ReorderList>> {
        let control = self.world().pointer_press(document, pointer_id)?;
        let entity = self.reorder_list_entity(self.nearest_reorder_list(control)?)?;
        self.read(entity, |list| {
            list.armed
                .as_ref()
                .is_some_and(|armed| armed.pointer_id == pointer_id && armed.control == control)
        })
        .ok()?
        .then_some(entity)
    }

    fn reorder_row_boxes(&self, id: StableNodeId, bounds: LayoutBox) -> Vec<LayoutBox> {
        let children = self
            .world()
            .node(id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        if children.is_empty() {
            let metrics = self.world().theme_metrics();
            return self
                .read(crate::Entity::<ReorderList>::from_stable_id(id), |list| {
                    list.row_bounds(bounds, metrics)
                })
                .unwrap_or_default();
        }
        children
            .into_iter()
            .filter_map(|child| self.world().component_layout_box(child))
            .collect()
    }

    fn reorder_tool_boxes(&self, entity: crate::Entity<ReorderList>) -> Vec<LayoutBox> {
        let tools = self
            .read(entity, |list| {
                list.items
                    .iter()
                    .filter_map(|item| item.tools)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        tools
            .into_iter()
            .filter_map(|id| self.world().component_layout_box(id))
            .collect()
    }

    /// Resolve a secondary press on `id` to the row body under the point and
    /// publish [`ReorderListEvent::Secondary`] on the list. Returns `Ok(false)`
    /// when `id` is not a reorder list, the point sits on a row's tool cluster
    /// (tools keep their own pointer handling) or on empty space, so the
    /// secondary-press walk keeps bubbling.
    pub fn emit_reorder_row_secondary(
        &mut self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, crate::FrameworkError> {
        if !point_finite(x, y) || !self.is_reorder_list(id) {
            return Ok(false);
        }
        let Some(entity) = self.reorder_list_entity(id) else {
            return Ok(false);
        };
        let Some(bounds) = self.world().component_layout_box(id) else {
            return Ok(false);
        };
        // The row is found in layout coordinates; the event keeps the window
        // point, which is where a context menu opens.
        let Some((layout_x, layout_y)) = self.world().pointer_layout_position(id, x, y) else {
            return Ok(false);
        };
        let rows = self.reorder_row_boxes(id, bounds);
        let exclude = self.reorder_tool_boxes(entity);
        if exclude
            .iter()
            .any(|reserved| reserved.contains(layout_x, layout_y))
        {
            return Ok(false);
        }
        let (values, enabled) = self
            .read(entity, |list| {
                (
                    list.items
                        .iter()
                        .map(|item| Arc::clone(&item.value))
                        .collect::<Vec<_>>(),
                    list.items
                        .iter()
                        .map(|item| !item.disabled)
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or((Vec::new(), Vec::new()));
        let Some(index) = item_at(&rows, &enabled, layout_x, layout_y) else {
            return Ok(false);
        };
        let source = Arc::clone(&values[index]);
        self.update_component(entity, |_, cx| {
            cx.emit(ReorderListEvent::Secondary { source, x, y });
        })?;
        Ok(true)
    }
}

fn point_finite(x: f32, y: f32) -> bool {
    x.is_finite() && y.is_finite()
}

fn item_at(bounds: &[LayoutBox], enabled: &[bool], x: f32, y: f32) -> Option<usize> {
    bounds.iter().enumerate().find_map(|(index, row)| {
        (enabled.get(index).copied().unwrap_or(false) && row.contains(x, y)).then_some(index)
    })
}

fn reorder_insert_line(
    list_bounds: LayoutBox,
    rows: &[LayoutBox],
    before: Option<usize>,
) -> LayoutBox {
    let y = before
        .and_then(|index| rows.get(index).map(|row| row.y - INSERT_THICKNESS))
        .or_else(|| rows.last().map(|row| row.y + row.height + INSERT_THICKNESS))
        .unwrap_or(list_bounds.y);
    LayoutBox {
        x: list_bounds.x + INSERT_INSET,
        y,
        width: (list_bounds.width - INSERT_INSET * 2.0).max(0.0),
        height: INSERT_THICKNESS,
    }
}

/// The mark for `position` against row `target`; a line starts `indent` in.
fn tree_insert_line(target: LayoutBox, position: TreeDropPosition, indent: f32) -> LayoutBox {
    let indent = indent.min((target.width - INSERT_INSET * 2.0).max(0.0));
    match position {
        TreeDropPosition::Before => LayoutBox {
            x: target.x + INSERT_INSET + indent,
            y: target.y - 1.0,
            width: (target.width - INSERT_INSET * 2.0 - indent).max(0.0),
            height: INSERT_THICKNESS,
        },
        TreeDropPosition::After => LayoutBox {
            x: target.x + INSERT_INSET + indent,
            y: target.y + target.height - 1.0,
            width: (target.width - INSERT_INSET * 2.0 - indent).max(0.0),
            height: INSERT_THICKNESS,
        },
        TreeDropPosition::Inside => LayoutBox {
            x: target.x + 3.0,
            y: target.y + INSERT_THICKNESS,
            width: (target.width - 6.0).max(0.0),
            height: (target.height - 4.0).max(0.0),
        },
    }
}

#[cfg(test)]
#[path = "reorder_list/preview_tests.rs"]
mod preview_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn sample() -> ReorderList {
        ReorderList::new([
            ReorderItem::new("a", "Alpha"),
            ReorderItem::new("b", "Beta"),
            ReorderItem::new("c", "Gamma"),
        ])
    }

    fn bounds() -> LayoutBox {
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 180.0,
            height: 3.0 * ControlSize::Small.height_in(UI_METRICS) + 2.0 * DEFAULT_SPACING,
        }
    }

    fn apply(list: &mut ReorderList, pointer: ReorderListPointer) -> Option<ReorderListEvent> {
        list.apply_pointer(pointer, bounds(), UI_METRICS)
    }

    #[test]
    fn click_without_move_selects() {
        let mut list = sample();
        assert_eq!(
            apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 42.0 }),
            None
        );
        assert!(list.is_dragging());
        assert_eq!(
            apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 45.0 }),
            None
        );
        assert!(list.insert_line(bounds(), UI_METRICS).is_none());
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 45.0 }),
            Some(ReorderListEvent::Select(Arc::from("b")))
        );
        assert_eq!(list.selected_value().map(Arc::as_ref), Some("b"));
        assert!(!list.is_dragging());
    }

    #[test]
    fn drag_past_threshold_reorders_with_before_value() {
        let mut list = sample();
        apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 12.0 });
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 80.0 });
        // Dropping past the last row puts the line one thickness below the
        // whole stack. Derived from the same constants `bounds()` is, so a
        // spacing or control-height token can move without editing a literal.
        assert_eq!(
            list.insert_line(bounds(), UI_METRICS),
            Some(LayoutBox {
                x: bounds().x + INSERT_INSET,
                y: bounds().y + bounds().height + INSERT_THICKNESS,
                width: bounds().width - INSERT_INSET * 2.0,
                height: INSERT_THICKNESS,
            })
        );
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 80.0 }),
            Some(ReorderListEvent::Reorder {
                source: Arc::from("a"),
                before: None,
            })
        );
        assert_eq!(
            list.items
                .iter()
                .map(|item| item.value.as_ref())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert!(!list.is_dragging());

        let mut list = sample();
        apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 72.0 });
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 12.0 });
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 12.0 }),
            Some(ReorderListEvent::Reorder {
                source: Arc::from("c"),
                before: Some(Arc::from("a")),
            })
        );
    }

    #[test]
    fn a_list_that_does_not_reorder_only_selects() {
        let mut list = sample().reorder(false);
        apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 72.0 });
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 12.0 });
        assert_eq!(list.drag_visual(), None);
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 12.0 }),
            Some(ReorderListEvent::Select(Arc::from("c")))
        );
        assert_eq!(list.key_move("c", ReorderKeyMove::Up), None);
    }

    #[test]
    fn a_row_that_does_not_move_only_selects() {
        let mut list = sample();
        list.items[2] = list.items[2].clone().movable(false);
        apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 72.0 });
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 12.0 });
        assert_eq!(list.drag_visual(), None);
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 12.0 }),
            Some(ReorderListEvent::Select(Arc::from("c")))
        );
        assert_eq!(list.key_move("c", ReorderKeyMove::Up), None);
        // the other rows still drag
        apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 12.0 });
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 72.0 });
        assert!(list.drag_visual().is_some());
    }

    #[test]
    fn tree_drop_on_drop_only_row_is_inside() {
        let mut list = ReorderList::new([
            ReorderItem::new("a", "Alpha"),
            ReorderItem::new("b", "Beta")
                .draggable(false)
                .drop_target(true),
            ReorderItem::new("c", "Gamma"),
        ])
        .tree_drop(true);
        apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 12.0 });
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 42.0 });
        // The inside highlight sits on the second row, inset within it, and
        // is told apart from a line between rows.
        let rows = list.row_bounds(bounds(), UI_METRICS);
        assert!(matches!(
            list.drag_visual().unwrap().drop_mark(bounds(), &rows),
            Some(ReorderDropMark::Inside(_))
        ));
        let row = ControlSize::Small.height_in(UI_METRICS);
        assert_eq!(
            list.insert_line(bounds(), UI_METRICS),
            Some(LayoutBox {
                x: bounds().x + 3.0,
                y: bounds().y + row + DEFAULT_SPACING + INSERT_THICKNESS,
                width: bounds().width - 6.0,
                height: row - 4.0,
            })
        );
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 42.0 }),
            Some(ReorderListEvent::TreeDrop {
                source: Arc::from("a"),
                intent: TreeDropIntent {
                    target: Arc::from("b"),
                    position: TreeDropPosition::Inside,
                },
            })
        );
    }

    #[test]
    fn a_row_that_takes_nothing_inside_splits_into_before_and_after() {
        let list = || {
            ReorderList::new([
                ReorderItem::new("a", "Alpha"),
                ReorderItem::new("b", "Beta").accepts_inside(ReorderAccepts::Nothing),
                ReorderItem::new("c", "Gamma"),
                ReorderItem::new("d", "Delta"),
            ])
            .tree_drop(true)
        };
        let row = ControlSize::Small.height_in(UI_METRICS);
        let top = row + DEFAULT_SPACING;
        for (offset, position) in [
            (0.4, TreeDropPosition::Before),
            (0.6, TreeDropPosition::After),
        ] {
            let mut list = list();
            let y = top + row * offset;
            let last = 3.0 * top + 12.0;
            apply(&mut list, ReorderListPointer::Down { x: 40.0, y: last });
            apply(&mut list, ReorderListPointer::Move { x: 40.0, y });
            assert_eq!(
                apply(&mut list, ReorderListPointer::Up { x: 40.0, y }),
                Some(ReorderListEvent::TreeDrop {
                    source: Arc::from("d"),
                    intent: TreeDropIntent {
                        target: Arc::from("b"),
                        position,
                    },
                }),
                "a drop at {offset} of the row"
            );
        }
    }

    /// `rows` as `(value, depth, kind)`, a tree drop list over uniform rows.
    fn tree(rows: &[(&str, u16, &str)]) -> ReorderList {
        ReorderList::new(
            rows.iter().map(|(value, depth, kind)| {
                ReorderItem::new(*value, *value).depth(*depth).kind(*kind)
            }),
        )
        .tree_drop(true)
    }

    /// The uniform rows of `list`, tall enough for every row.
    fn tree_rows(list: &ReorderList) -> Vec<LayoutBox> {
        list.row_bounds(
            LayoutBox {
                height: 400.0,
                ..bounds()
            },
            UI_METRICS,
        )
    }

    /// A drag of `source` to `(x, y)` over `list`: its drag visual, then its
    /// release's event.
    fn drag_to(
        list: &mut ReorderList,
        source: usize,
        x: f32,
        y: f32,
    ) -> (Option<ReorderDragVisual>, Option<ReorderListEvent>) {
        let rows = tree_rows(list);
        let start = rows[source];
        let press = ReorderListPointer::Down {
            x: 40.0,
            y: start.y + start.height * 0.5,
        };
        list.apply_pointer_with_rows(press, &rows, &[]);
        list.apply_pointer_with_rows(ReorderListPointer::Move { x, y }, &rows, &[]);
        let visual = list.drag_visual();
        let event = list.apply_pointer_with_rows(ReorderListPointer::Up { x, y }, &rows, &[]);
        (visual, event)
    }

    fn tree_drop(source: &str, target: &str, position: TreeDropPosition) -> ReorderListEvent {
        ReorderListEvent::TreeDrop {
            source: Arc::from(source),
            intent: TreeDropIntent {
                target: Arc::from(target),
                position,
            },
        }
    }

    #[test]
    fn a_dragged_row_carries_the_rows_it_holds_and_never_lands_among_them() {
        let mut list = tree(&[
            ("group", 0, "group"),
            ("a", 1, "content"),
            ("inner", 1, "group"),
            ("b", 2, "content"),
            ("x", 0, "content"),
        ]);
        let rows = tree_rows(&list);
        // Over its own rows, the middle of each: nowhere to land.
        for held in 1..4 {
            let row = rows[held];
            let (visual, event) = drag_to(&mut list, 0, 40.0, row.y + row.height * 0.5);
            let visual = visual.expect("a moved drag");
            assert_eq!(visual.carried(), Some(0..=3));
            assert_eq!(visual.drop_mark(bounds(), &rows), None);
            assert_eq!(event, None, "a drop on held row {held}");
        }
        // The carried rows are outlined as one block.
        let row = rows[4];
        let (visual, event) = drag_to(&mut list, 0, 40.0, row.y + row.height * 0.5);
        assert_eq!(
            visual.unwrap().carried_box(&rows),
            Some(LayoutBox {
                height: rows[3].y + rows[3].height - rows[0].y,
                ..rows[0]
            })
        );
        assert_eq!(
            event,
            Some(tree_drop("group", "x", TreeDropPosition::Inside))
        );
        // A row deeper in carries only what it holds.
        let (visual, _) = drag_to(&mut list, 2, 40.0, row.y + row.height * 0.5);
        assert_eq!(visual.unwrap().carried(), Some(2..=3));
    }

    #[test]
    fn a_row_takes_inside_only_the_kinds_it_accepts() {
        let rows = || {
            ReorderList::new([
                ReorderItem::new("image", "image")
                    .kind("content")
                    .accepts_inside(ReorderAccepts::kinds(["effect", "mask"])),
                ReorderItem::new("glow", "glow")
                    .depth(1)
                    .kind("effect")
                    .accepts_inside(ReorderAccepts::kinds(["mask"])),
                ReorderItem::new("mask", "mask")
                    .depth(2)
                    .kind("mask")
                    .accepts_inside(ReorderAccepts::Nothing),
                ReorderItem::new("loose", "loose").kind("mask"),
                ReorderItem::new("photo", "photo").kind("content"),
            ])
            .tree_drop(true)
        };
        let middle = |list: &ReorderList, row: usize| {
            let row = tree_rows(list)[row];
            row.y + row.height * 0.5
        };
        // A mask goes into the effect.
        let mut list = rows();
        let y = middle(&list, 1);
        assert_eq!(
            drag_to(&mut list, 3, 40.0, y).1,
            Some(tree_drop("loose", "glow", TreeDropPosition::Inside))
        );
        // Content does not: over the effect's middle it splits into halves,
        // and beside it is the image, which takes no content.
        let mut list = rows();
        let y = middle(&list, 1);
        let (visual, event) = drag_to(&mut list, 4, 40.0, y);
        assert_eq!(visual.unwrap().drop_mark(bounds(), &tree_rows(&list)), None);
        assert_eq!(event, None);
        // Nothing goes into a mask, nor content beside it in the effect, nor
        // after the effect in the image: only out, after the image.
        let mut list = rows();
        let y = middle(&list, 2);
        assert_eq!(
            drag_to(&mut list, 4, 40.0, y).1,
            Some(tree_drop("photo", "image", TreeDropPosition::After))
        );
        let mut list = rows();
        let y = middle(&list, 2) + 1.0;
        assert_eq!(
            drag_to(&mut list, 3, 40.0, y).1,
            Some(tree_drop("loose", "mask", TreeDropPosition::After))
        );
    }

    #[test]
    fn rows_that_take_no_siblings_and_a_root_that_takes_no_kind() {
        let list = || {
            ReorderList::new([
                ReorderItem::new("actor", "actor").kind("actor"),
                ReorderItem::new("pin", "pin")
                    .depth(1)
                    .kind("pinned")
                    .accepts_sibling(ReorderAccepts::Nothing),
                ReorderItem::new("effect", "effect").kind("effect"),
                ReorderItem::new("image", "image").kind("content"),
            ])
            .tree_drop(true)
        };
        let mut pinned = list();
        let pin = tree_rows(&pinned)[1];
        assert_eq!(drag_to(&mut pinned, 3, 40.0, pin.y + 2.0).1, None);
        // The root takes no effects: an effect may not go beside the image.
        let mut list = list().accepts_root(ReorderAccepts::kinds(["content", "actor"]));
        let image = tree_rows(&list)[3];
        assert_eq!(
            drag_to(&mut list, 2, 40.0, image.y + image.height - 2.0).1,
            None
        );
    }

    #[test]
    fn a_tree_line_is_indented_to_the_depth_it_lands_at() {
        let mut list = tree(&[
            ("group", 0, "group"),
            ("a", 1, "content"),
            ("x", 0, "content"),
        ])
        .indent(20.0);
        let rows = tree_rows(&list);
        let (visual, event) = drag_to(&mut list, 2, 40.0, rows[1].y + 2.0);
        assert_eq!(event, Some(tree_drop("x", "a", TreeDropPosition::Before)));
        assert_eq!(
            visual.unwrap().drop_mark(bounds(), &rows),
            Some(ReorderDropMark::Line(LayoutBox {
                x: rows[1].x + INSERT_INSET + 20.0,
                y: rows[1].y - 1.0,
                width: rows[1].width - INSERT_INSET * 2.0 - 20.0,
                height: INSERT_THICKNESS,
            }))
        );
        // The bottom of a row whose children show below it is the top of
        // those children.
        let (visual, event) = drag_to(&mut list, 2, 40.0, rows[0].y + rows[0].height - 2.0);
        assert_eq!(event, Some(tree_drop("x", "a", TreeDropPosition::Before)));
        assert!(matches!(
            visual.unwrap().drop_mark(bounds(), &rows),
            Some(ReorderDropMark::Line(line)) if line.x == rows[1].x + INSERT_INSET + 20.0
        ));
    }

    #[test]
    fn after_the_last_nested_row_the_pointer_picks_how_far_out() {
        let list = || {
            tree(&[
                ("group", 0, "group"),
                ("a", 1, "content"),
                ("x", 0, "content"),
                ("source", 0, "content"),
            ])
            .indent(20.0)
        };
        let a = tree_rows(&list())[1];
        let y = a.y + a.height - 2.0;
        // Over the row's text: at its own depth, inside the group.
        let mut deep = list();
        assert_eq!(
            drag_to(&mut deep, 3, 120.0, y).1,
            Some(tree_drop("source", "a", TreeDropPosition::After))
        );
        // Left of its indent: one level out, after the group.
        let mut out = list();
        let (visual, event) = drag_to(&mut out, 3, a.x + 4.0, y);
        assert_eq!(
            event,
            Some(tree_drop("source", "group", TreeDropPosition::After))
        );
        assert!(matches!(
            visual.unwrap().drop_mark(bounds(), &tree_rows(&list())),
            Some(ReorderDropMark::Line(line)) if line.x == a.x + INSERT_INSET
        ));
    }

    #[test]
    fn a_drop_that_leaves_the_row_where_it_is_changes_nothing() {
        let mut list = tree(&[("a", 0, "c"), ("b", 0, "c"), ("c", 0, "c")]);
        let rows = tree_rows(&list);
        // Right before its next sibling, and right after its previous one.
        assert_eq!(drag_to(&mut list, 1, 40.0, rows[2].y + 2.0).1, None);
        assert_eq!(
            drag_to(&mut list, 1, 40.0, rows[0].y + rows[0].height - 2.0).1,
            None
        );
    }

    #[test]
    fn the_keyboard_moves_a_row_past_its_siblings_and_in_and_out() {
        let list = tree(&[
            ("group", 0, "group"),
            ("a", 1, "content"),
            ("b", 1, "content"),
            ("x", 0, "content"),
            ("y", 0, "content"),
        ]);
        let moved = |source, step| list.key_move(source, step);
        assert_eq!(
            moved("x", ReorderKeyMove::Up),
            Some(tree_drop("x", "group", TreeDropPosition::Before))
        );
        assert_eq!(
            moved("group", ReorderKeyMove::Down),
            Some(tree_drop("group", "x", TreeDropPosition::After))
        );
        assert_eq!(moved("a", ReorderKeyMove::Up), None, "first of its parent");
        assert_eq!(moved("y", ReorderKeyMove::Down), None, "last of the list");
        // Into the sibling above, after its last child; out after its parent.
        assert_eq!(
            moved("x", ReorderKeyMove::Indent),
            Some(tree_drop("x", "b", TreeDropPosition::After))
        );
        assert_eq!(
            moved("y", ReorderKeyMove::Indent),
            Some(tree_drop("y", "x", TreeDropPosition::Inside))
        );
        assert_eq!(
            moved("b", ReorderKeyMove::Outdent),
            Some(tree_drop("b", "group", TreeDropPosition::After))
        );
        assert_eq!(moved("x", ReorderKeyMove::Outdent), None);
        // The rules a drag keeps hold for the keyboard too.
        let typed = ReorderList::new([
            ReorderItem::new("mask", "mask")
                .kind("mask")
                .accepts_inside(ReorderAccepts::Nothing),
            ReorderItem::new("image", "image").kind("content"),
        ])
        .tree_drop(true);
        assert_eq!(typed.key_move("image", ReorderKeyMove::Indent), None);

        // A flat list reorders by the row it goes before.
        let flat = sample();
        assert_eq!(
            flat.key_move("b", ReorderKeyMove::Up),
            Some(ReorderListEvent::Reorder {
                source: Arc::from("b"),
                before: Some(Arc::from("a")),
            })
        );
        assert_eq!(
            flat.key_move("b", ReorderKeyMove::Down),
            Some(ReorderListEvent::Reorder {
                source: Arc::from("b"),
                before: None,
            })
        );
        assert_eq!(flat.key_move("b", ReorderKeyMove::Indent), None);
    }

    #[test]
    fn alt_with_an_arrow_on_a_focused_list_moves_the_chosen_row() {
        use nana_ui_input::{
            InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey,
        };

        let mut context = AppContext::new();
        let document = document();
        let list = context
            .create_component(
                document,
                ReorderList::new([
                    ReorderItem::new("a", "a"),
                    ReorderItem::new("b", "b").selected(true),
                ])
                .tree_drop(true),
            )
            .unwrap();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        context
            .on(list, move |_, event: &ReorderListEvent, _| {
                observed.lock().unwrap().push(event.clone());
            })
            .unwrap();
        context.focus_node(document, list.stable_id()).unwrap();
        let mut input = crate::HeadlessInput::bind(&mut context, document);
        let key = |alt| {
            InputPayload::Key(KeyInput {
                physical: PhysicalKey("ArrowUp".into()),
                logical: LogicalKey("ArrowUp".into()),
                state: KeyState::Pressed,
                repeat: false,
                modifiers: InputModifiers {
                    alt,
                    ..InputModifiers::empty()
                },
            })
        };
        let outcome = input.route(&mut context, key(true)).unwrap();
        assert!(outcome.handled);
        assert_eq!(
            std::mem::take(&mut *events.lock().unwrap()),
            [tree_drop("b", "a", TreeDropPosition::Before)]
        );
        input.route(&mut context, key(false)).unwrap();
        assert_eq!(
            std::mem::take(&mut *events.lock().unwrap()),
            Vec::<ReorderListEvent>::new()
        );
    }

    #[test]
    fn invalid_tree_drop_does_not_emit_reorder() {
        let mut list = sample().tree_drop(true);
        apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 12.0 });
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 90.0 });
        assert!(list.insert_line(bounds(), UI_METRICS).is_none());
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 90.0 }),
            None
        );

        let mut list = sample();
        apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 12.0 });
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 90.0 });
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 90.0 }),
            Some(ReorderListEvent::Reorder {
                source: Arc::from("a"),
                before: None,
            })
        );
    }

    #[test]
    fn disabled_and_non_draggable_sources_are_ignored() {
        let mut list = ReorderList::new([
            ReorderItem::new("a", "Alpha"),
            ReorderItem::new("b", "Beta").disabled(true),
            ReorderItem::new("c", "Gamma").draggable(false),
        ]);
        assert_eq!(
            apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 42.0 }),
            None
        );
        assert!(!list.is_dragging());
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 80.0 });
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 80.0 }),
            None
        );

        assert_eq!(
            apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 72.0 }),
            None
        );
        assert!(!list.is_dragging());
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 12.0 });
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 12.0 }),
            None
        );
    }

    #[test]
    fn reserved_tool_boxes_do_not_begin_a_drag() {
        let mut list = sample();
        let rows = list.row_bounds(bounds(), UI_METRICS);
        let exclude = [LayoutBox {
            x: 120.0,
            y: 0.0,
            width: 60.0,
            height: 28.0,
        }];
        assert_eq!(
            list.apply_pointer_with_rows(
                ReorderListPointer::Down { x: 140.0, y: 12.0 },
                &rows,
                &exclude
            ),
            None
        );
        assert!(!list.is_dragging());
        assert_eq!(
            list.apply_pointer_with_rows(
                ReorderListPointer::Down { x: 40.0, y: 12.0 },
                &rows,
                &exclude
            ),
            None
        );
        assert!(list.is_dragging());
    }

    #[test]
    fn cancel_clears_transient_drag_state() {
        let mut list = sample();
        apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 12.0 });
        apply(&mut list, ReorderListPointer::Move { x: 40.0, y: 80.0 });
        assert!(list.is_dragging());
        assert!(list.insert_line(bounds(), UI_METRICS).is_some());
        assert_eq!(
            apply(&mut list, ReorderListPointer::Cancel),
            Some(ReorderListEvent::Cancelled)
        );
        assert!(!list.is_dragging());
        assert!(list.insert_line(bounds(), UI_METRICS).is_none());
        assert_eq!(list.cancel(), None);
        assert_eq!(
            apply(&mut list, ReorderListPointer::Up { x: 40.0, y: 80.0 }),
            None
        );
    }

    #[test]
    fn projects_a_pointer_focusable_list_surface() {
        let mut context = AppContext::new();
        let list = context.create_component(document(), sample()).unwrap();
        let id = list.stable_id();
        assert!(matches!(
            context.world().node(id).map(|node| node.kind),
            Some(NodeKind::Element { tag }) if tag == "reorder-list"
        ));
        assert_eq!(
            context.world().interaction(id),
            Some(InteractionState {
                pointer_events: true,
                focusable: true,
            })
        );
        assert_eq!(
            context.world().widget_visual_as::<ReorderListVisual>(id),
            Some(&ReorderListVisual {
                rows: sample().paint_rows(),
                size: ControlSize::Small,
                spacing: DEFAULT_SPACING,
                drag: None,
            })
        );
        let style = context.world().node_style(id).expect("projected style");
        assert_eq!(style.layout.width, Some(LengthSpec::Fill));
        assert_eq!(
            style.layout.height,
            Some(LengthSpec::Px(
                3.0 * ControlSize::Small.height_in(UI_METRICS) + 2.0 * DEFAULT_SPACING
            ))
        );
    }

    #[test]
    fn an_installed_compact_height_reaches_self_drawn_reorder_hit_rows() {
        let mut context = AppContext::new();
        let list = context.create_component(document(), sample()).unwrap();
        context
            .layout_document(document(), crate::LayoutViewport::new(180.0, 200.0))
            .unwrap();
        let mut metrics = nana_ui_core::UI_METRICS;
        metrics.compact_control_height = 40.0;
        assert!(
            context
                .set_style_tokens(
                    nana_ui_core::ThemeAppearance::Dark,
                    metrics,
                    nana_ui_core::SemanticPalette::dark(),
                    nana_ui_core::SemanticPalette::dark().surface,
                )
                .unwrap()
        );
        // The box layout wrote: new metrics suspend the published result
        // until the next layout, and the press lands before that.
        let bounds = context.world().layout_box(list.stable_id()).unwrap();
        // Compile-time Small rows are 28px; at y=35 that is row "b". Installed
        // 40px rows put the same point on row "a".
        let x = bounds.x + 40.0;
        let y = bounds.y + 35.0;
        assert!(
            context
                .begin_reorder_list_pointer(document(), 1, list.stable_id(), x, y)
                .unwrap()
        );
        assert!(
            context
                .end_reorder_list_pointer(document(), 1, x, y, false)
                .unwrap()
        );
        assert_eq!(
            context
                .read(list, |list| list.selected_value().map(Arc::clone))
                .unwrap()
                .as_deref(),
            Some("a")
        );
    }

    #[test]
    fn a_moved_tree_drop_over_live_rows_outlines_its_row_and_shows_where_it_lands() {
        let mut context = AppContext::new();
        let document = document();
        // A flat drag over live rows previews on the rows instead; a tree
        // drop keeps the marks (see `preview_tests`).
        let list = context
            .create_component(
                document,
                sample().live_rows(true).tree_drop(true).spacing(4.0),
            )
            .unwrap();
        // Live rows taller than the self-drawn ones, so a box taken from the
        // uniform rows would land in the wrong place.
        let mut rows = Vec::new();
        for _ in 0..3 {
            let row = context
                .create_detached_component(
                    document,
                    crate::Stack::column(0.0).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Fill);
                        layout.height = Some(LengthSpec::Px(40.0));
                    }),
                )
                .unwrap();
            context.append_child(list, row).unwrap();
            rows.push(row.stable_id());
        }
        context
            .layout_document(document, crate::LayoutViewport::new(200.0, 300.0))
            .unwrap();
        let boxes: Vec<_> = rows
            .iter()
            .map(|row| context.world().layout_box(*row).unwrap())
            .collect();
        let geometry = |context: &AppContext| {
            let layout = context
                .world()
                .reorder_list_layout(list.stable_id())
                .expect("reorder list layout");
            (
                layout.insert.map(|(line, _)| line),
                layout.dragged.map(|(row, _)| row),
            )
        };
        assert_eq!(geometry(&context), (None, None));

        let x = boxes[0].x + 20.0;
        let start = boxes[0].y + 20.0;
        assert!(
            context
                .begin_reorder_list_pointer(document, 1, list.stable_id(), x, start)
                .unwrap()
        );
        // Under the threshold: nothing is dragged yet.
        context
            .update_reorder_list_pointer(document, 1, x, start + 2.0)
            .unwrap();
        assert_eq!(geometry(&context), (None, None));
        // In the bottom quarter of the last row: it goes after it.
        let end = boxes[2].y + 38.0;
        context
            .update_reorder_list_pointer(document, 1, x, end)
            .unwrap();
        let (insert, dragged) = geometry(&context);
        assert_eq!(dragged, Some(boxes[0]));
        let insert = insert.expect("an insert line");
        assert!(
            insert.y + insert.height >= boxes[2].y + boxes[2].height,
            "the line {insert:?} sits at the bottom of the last live row {:?}",
            boxes[2]
        );
        context
            .end_reorder_list_pointer(document, 1, x, end, false)
            .unwrap();
        assert_eq!(geometry(&context), (None, None));
    }

    #[test]
    fn declared_live_rows_suppress_self_painted_rows() {
        let mut context = AppContext::new();
        let list = context
            .create_component(document(), sample().live_rows(true))
            .unwrap();
        let id = list.stable_id();
        assert_eq!(
            context.world().widget_visual_as::<ReorderListVisual>(id),
            Some(&ReorderListVisual {
                rows: Arc::from([]),
                size: ControlSize::Small,
                spacing: DEFAULT_SPACING,
                drag: None,
            })
        );
        let style = context.world().node_style(id).expect("projected style");
        assert_eq!(style.layout.height, Some(LengthSpec::Shrink));
    }

    /// A live list of rows `a`, `b`, `c`: each row a 28px strip that is not
    /// hittable itself, with a 60px button at its start. The button's
    /// clicks and the list's events are recorded.
    struct ButtonRows {
        context: AppContext,
        list: crate::Entity<ReorderList>,
        buttons: Vec<crate::Entity<crate::Button>>,
        rows: Vec<LayoutBox>,
        events: Arc<std::sync::Mutex<Vec<ReorderListEvent>>>,
        clicks: Arc<std::sync::Mutex<Vec<usize>>>,
        input: crate::HeadlessInput,
    }

    impl ButtonRows {
        /// `tools` registers each row's button as the row's tools region.
        fn new(tools: bool) -> Self {
            use crate::{Activate, Button, Stack};
            use nana_ui_core::LayoutStyle;

            let document = document();
            let mut context = AppContext::new();
            let values = ["a", "b", "c"];
            let mut list_style = NodeStyle::default();
            Arc::make_mut(&mut list_style.layout).width = Some(LengthSpec::Px(200.0));
            let list = context
                .create_component(
                    document,
                    ReorderList::new(values.map(|value| ReorderItem::new(value, value)))
                        .live_rows(true)
                        .style(list_style),
                )
                .unwrap();
            let clicks = Arc::new(std::sync::Mutex::new(Vec::new()));
            let mut buttons = Vec::new();
            for index in 0..values.len() {
                let row = context
                    .create_component(
                        document,
                        Stack::row(0.0).with_layout(|layout| {
                            layout.width = Some(LengthSpec::Fill);
                            layout.height = Some(LengthSpec::Px(28.0));
                        }),
                    )
                    .unwrap();
                context.append_child(list, row).unwrap();
                let button = context
                    .create_component(
                        document,
                        Button::new("Play").layout(Arc::new(LayoutStyle {
                            width: Some(LengthSpec::Px(60.0)),
                            height: Some(LengthSpec::Px(28.0)),
                            ..LayoutStyle::default()
                        })),
                    )
                    .unwrap();
                context.append_child(row, button).unwrap();
                let observed = Arc::clone(&clicks);
                context
                    .on(button, move |_, _: &Activate, _| {
                        observed.lock().unwrap().push(index);
                    })
                    .unwrap();
                buttons.push(button);
            }
            if tools {
                let items = values
                    .iter()
                    .zip(&buttons)
                    .map(|(value, button)| {
                        ReorderItem::new(*value, *value).tools(button.stable_id())
                    })
                    .collect::<Vec<_>>();
                context
                    .update_component(list, |list, _| list.items = items)
                    .unwrap();
            }
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = Arc::clone(&events);
            context
                .on(list, move |_, event: &ReorderListEvent, _| {
                    observed.lock().unwrap().push(event.clone());
                })
                .unwrap();
            context
                .layout_document(document, crate::LayoutViewport::new(400.0, 300.0))
                .unwrap();
            context.rebuild_hit_test(document);
            let rows = context
                .world()
                .node(list.stable_id())
                .unwrap()
                .children
                .iter()
                .map(|row| context.world().layout_box(*row).unwrap())
                .collect();
            let input = crate::HeadlessInput::bind(&mut context, document);
            Self {
                context,
                list,
                buttons,
                rows,
                events,
                clicks,
                input,
            }
        }

        fn pointer(&mut self, phase: nana_ui_input::PointerPhase, x: f32, y: f32) {
            self.input.pointer(&mut self.context, phase, x, y).unwrap();
        }

        /// The middle of row `index`'s button.
        fn on_button(&self, index: usize) -> (f32, f32) {
            let row = self.rows[index];
            (row.x + 30.0, row.y + row.height * 0.5)
        }

        /// The middle of row `index`'s body, right of its button.
        fn on_body(&self, index: usize) -> (f32, f32) {
            let row = self.rows[index];
            (row.x + 150.0, row.y + row.height * 0.5)
        }

        fn dragging(&self) -> bool {
            self.context
                .read(self.list, ReorderList::is_dragging)
                .unwrap()
        }

        fn captured(&self) -> bool {
            !self.context.world().pointer_captures(document()).is_empty()
        }

        /// The dragged row's index, while the list paints a drag.
        fn painted_drag(&self) -> Option<usize> {
            self.context
                .read(self.list, ReorderList::drag_visual)
                .unwrap()
                .and_then(|visual| visual.source)
        }

        fn take_events(&self) -> Vec<ReorderListEvent> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }

        fn take_clicks(&self) -> Vec<usize> {
            std::mem::take(&mut *self.clicks.lock().unwrap())
        }
    }

    #[test]
    fn a_button_in_a_row_keeps_its_click_until_the_press_moves() {
        use nana_ui_input::PointerPhase;

        let mut rows = ButtonRows::new(false);
        assert_eq!(rows.buttons.len(), 3);
        // Pressed and released in place: the button's click, not the row's.
        let (x, y) = rows.on_button(0);
        rows.pointer(PointerPhase::Down, x, y);
        assert!(
            !rows.dragging(),
            "a press on a button does not start a drag"
        );
        assert!(!rows.captured(), "nor take the pointer from the button");
        assert_eq!(
            rows.context.world().pointer_press(document(), 1),
            Some(rows.buttons[0].stable_id())
        );
        rows.pointer(PointerPhase::Up, x, y);
        assert_eq!(rows.take_clicks(), [0]);
        assert_eq!(rows.take_events(), Vec::<ReorderListEvent>::new());

        // A jitter under the threshold is still a click.
        rows.pointer(PointerPhase::Down, x, y);
        rows.pointer(PointerPhase::Move, x + 1.0, y + 2.0);
        assert!(!rows.dragging());
        assert_eq!(rows.painted_drag(), None);
        rows.pointer(PointerPhase::Up, x + 1.0, y + 2.0);
        assert_eq!(rows.take_clicks(), [0]);
        assert_eq!(rows.take_events(), Vec::<ReorderListEvent>::new());
    }

    #[test]
    fn a_press_on_a_button_that_moves_drags_its_row() {
        use nana_ui_input::PointerPhase;

        let mut rows = ButtonRows::new(false);
        let (x, y) = rows.on_button(0);
        rows.pointer(PointerPhase::Down, x, y);
        let below = rows.rows[2].y + rows.rows[2].height - 2.0;
        rows.pointer(PointerPhase::Move, x, below);
        // Past the threshold the list takes the pointer, and the button its
        // press back: the release is the drop, not a click.
        assert!(rows.dragging());
        assert!(rows.captured());
        assert_eq!(rows.painted_drag(), Some(0));
        assert_eq!(rows.context.world().pointer_press(document(), 1), None);
        assert_eq!(
            rows.context.world().focused(document()),
            Some(rows.list.stable_id())
        );
        rows.pointer(PointerPhase::Up, x, below);
        assert_eq!(
            rows.take_events(),
            [ReorderListEvent::Reorder {
                source: Arc::from("a"),
                before: None,
            }]
        );
        assert_eq!(rows.take_clicks(), Vec::<usize>::new());
        assert!(!rows.dragging());
        assert!(!rows.captured());

        // From the last row's button up to the first row.
        let (x, y) = rows.on_button(2);
        rows.pointer(PointerPhase::Down, x, y);
        let top = rows.rows[0].y + 2.0;
        rows.pointer(PointerPhase::Move, x, top);
        rows.pointer(PointerPhase::Up, x, top);
        assert_eq!(
            rows.take_events(),
            [ReorderListEvent::Reorder {
                source: Arc::from("c"),
                before: Some(Arc::from("a")),
            }]
        );
        assert_eq!(rows.take_clicks(), Vec::<usize>::new());
    }

    #[test]
    fn a_row_body_still_selects_and_drags_at_once() {
        use nana_ui_input::PointerPhase;

        let mut rows = ButtonRows::new(false);
        let (x, y) = rows.on_body(1);
        rows.pointer(PointerPhase::Down, x, y);
        assert!(rows.dragging(), "the row's own surface starts the gesture");
        assert!(rows.captured());
        rows.pointer(PointerPhase::Up, x, y);
        assert_eq!(
            rows.take_events(),
            [ReorderListEvent::Select(Arc::from("b"))]
        );
        assert_eq!(rows.take_clicks(), Vec::<usize>::new());

        let (x, y) = rows.on_body(0);
        rows.pointer(PointerPhase::Down, x, y);
        let below = rows.rows[2].y + rows.rows[2].height - 2.0;
        rows.pointer(PointerPhase::Move, x, below);
        rows.pointer(PointerPhase::Up, x, below);
        assert_eq!(
            rows.take_events(),
            [ReorderListEvent::Reorder {
                source: Arc::from("a"),
                before: None,
            }]
        );
    }

    #[test]
    fn a_tools_region_still_never_starts_a_drag() {
        use nana_ui_input::PointerPhase;

        let mut rows = ButtonRows::new(true);
        let (x, y) = rows.on_button(0);
        rows.pointer(PointerPhase::Down, x, y);
        let below = rows.rows[2].y + rows.rows[2].height - 2.0;
        rows.pointer(PointerPhase::Move, x, below);
        assert!(!rows.dragging());
        assert!(!rows.captured());
        rows.pointer(PointerPhase::Up, x, below);
        assert_eq!(rows.take_events(), Vec::<ReorderListEvent>::new());
        // Released off the button: no click either.
        assert_eq!(rows.take_clicks(), Vec::<usize>::new());

        rows.pointer(PointerPhase::Down, x, y);
        rows.pointer(PointerPhase::Up, x, y);
        assert_eq!(rows.take_clicks(), [0]);
        assert_eq!(rows.take_events(), Vec::<ReorderListEvent>::new());
    }

    #[test]
    fn secondary_press_on_a_row_body_publishes_the_row() {
        use std::sync::Mutex;

        let mut context = AppContext::new();
        let document = document();
        let mut style = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Px(200.0));
            layout.height = Some(LengthSpec::Px(48.0));
        }
        let list = context
            .create_component(
                document,
                ReorderList::new([
                    ReorderItem::new("a", "Alpha"),
                    ReorderItem::new("b", "Beta"),
                ])
                .size(ControlSize::Small)
                .style(style),
            )
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        context
            .on(list, move |_list, event: &ReorderListEvent, _cx| {
                observed.lock().unwrap().push(event.clone());
            })
            .unwrap();
        context
            .layout_document(document, crate::LayoutViewport::new(200.0, 48.0))
            .unwrap();
        context.rebuild_hit_test(document);

        // Row bodies are pointer-transparent; the secondary press still lands
        // on the list and resolves to the row under the point.
        assert_eq!(
            context.secondary_press_at(document, 100.0, 8.0).unwrap(),
            Some(list.stable_id())
        );
        assert_eq!(
            events.lock().unwrap().last().cloned(),
            Some(ReorderListEvent::Secondary {
                source: Arc::from("a"),
                x: 100.0,
                y: 8.0,
            })
        );
        assert_eq!(
            context.secondary_press_at(document, 100.0, 30.0).unwrap(),
            Some(list.stable_id())
        );
        assert_eq!(
            events.lock().unwrap().last().cloned(),
            Some(ReorderListEvent::Secondary {
                source: Arc::from("b"),
                x: 100.0,
                y: 30.0,
            })
        );
    }

    #[test]
    fn scrolled_reorder_inputs_keep_the_source_drop_and_secondary_anchor() {
        use crate::{Button, LayoutViewport, ScrollAxes, ScrollOffset, ScrollView};
        use std::sync::Mutex;
        for live in [false, true] {
            for control in [false, true] {
                if control && !live {
                    continue;
                }
                let mut cx = AppContext::new();
                let document = DocumentId::new(1).unwrap();
                let scroll = cx
                    .create_component(
                        document,
                        ScrollView::new(ScrollAxes::Vertical).with_layout(|l| {
                            l.width = Some(LengthSpec::Px(200.0));
                            l.height = Some(LengthSpec::Px(120.0));
                        }),
                    )
                    .unwrap();
                let list = cx
                    .create_detached_component(
                        document,
                        ReorderList::new(
                            (0..8)
                                .map(|i| ReorderItem::new(format!("row-{i}"), format!("Row {i}"))),
                        )
                        .live_rows(live),
                    )
                    .unwrap();
                cx.append_child(scroll, list).unwrap();
                if live {
                    for i in 0..8 {
                        let row = cx
                            .create_detached_component(
                                document,
                                crate::Stack::column(0.0).with_layout(|l| {
                                    l.height = Some(LengthSpec::Px(40.0));
                                    l.flex_shrink = Some(0.0);
                                }),
                            )
                            .unwrap();
                        let button = cx
                            .create_detached_component(
                                document,
                                Button::new(format!("Action {i}")).layout(Arc::new(
                                    nana_ui_core::LayoutStyle {
                                        width: Some(LengthSpec::Px(60.0)),
                                        ..Default::default()
                                    },
                                )),
                            )
                            .unwrap();
                        cx.append_child(row, button).unwrap();
                        cx.append_child(list, row).unwrap();
                    }
                }
                cx.layout_document(document, LayoutViewport::new(200.0, 120.0))
                    .unwrap();
                let list_bounds = cx.world().component_layout_box(list.stable_id()).unwrap();
                let rows = cx.reorder_row_boxes(list.stable_id(), list_bounds);
                cx.scroll_to(
                    scroll,
                    ScrollOffset {
                        x: 0.0,
                        y: rows[3].y,
                    },
                )
                .unwrap();
                let offset = cx.world().scroll_offset(scroll.stable_id()).unwrap().y;
                cx.rebuild_hit_test(document);
                let x = rows[4].x + if control { 10.0 } else { 150.0 };
                let source_y = rows[4].y + rows[4].height * 0.5 - offset;
                let target_y = rows[3].y + 1.0 - offset;
                let events = Arc::new(Mutex::new(Vec::new()));
                let received = Arc::clone(&events);
                cx.on(list, move |_, event: &ReorderListEvent, _| {
                    received.lock().unwrap().push(event.clone());
                })
                .unwrap();
                assert!(
                    cx.emit_reorder_row_secondary(list.stable_id(), x, source_y)
                        .unwrap()
                );
                assert_eq!(
                    events.lock().unwrap().as_slice(),
                    &[ReorderListEvent::Secondary {
                        source: "row-4".into(),
                        x,
                        y: source_y
                    }]
                );
                events.lock().unwrap().clear();
                let mut input = crate::HeadlessInput::bind(&mut cx, document);
                // A press on a row's button arms the drag and takes over once it
                // moves; a press on the row surface captures at once.
                input
                    .pointer(&mut cx, nana_ui_input::PointerPhase::Down, x, source_y)
                    .unwrap();
                input
                    .pointer(&mut cx, nana_ui_input::PointerPhase::Move, x, target_y)
                    .unwrap();
                assert_eq!(
                    cx.read(list, |v| v.drag_visual().and_then(|v| v.source))
                        .unwrap(),
                    Some(4)
                );
                input
                    .pointer(&mut cx, nana_ui_input::PointerPhase::Up, x, target_y)
                    .unwrap();
                assert_eq!(
                    events.lock().unwrap().as_slice(),
                    &[ReorderListEvent::Reorder {
                        source: "row-4".into(),
                        before: Some("row-3".into())
                    }]
                );
                events.lock().unwrap().clear();
                // Escape releases the capture too; the release that follows stays
                // cancelled and the next drag still commits.
                input
                    .pointer(&mut cx, nana_ui_input::PointerPhase::Down, x, source_y)
                    .unwrap();
                input
                    .pointer(&mut cx, nana_ui_input::PointerPhase::Move, x, target_y)
                    .unwrap();
                input
                    .press(
                        &mut cx,
                        nana_ui_input::KeyInput::named(
                            "Escape",
                            "Escape",
                            nana_ui_input::KeyState::Pressed,
                            Default::default(),
                        ),
                        None,
                        None,
                    )
                    .unwrap();
                input
                    .pointer(&mut cx, nana_ui_input::PointerPhase::Up, x, target_y)
                    .unwrap();
                assert_eq!(
                    events.lock().unwrap().as_slice(),
                    &[ReorderListEvent::Cancelled]
                );
                assert!(cx.world().pointer_captures(document).is_empty());
                events.lock().unwrap().clear();
                for (phase, y) in [
                    (nana_ui_input::PointerPhase::Down, source_y),
                    (nana_ui_input::PointerPhase::Move, target_y),
                    (nana_ui_input::PointerPhase::Up, target_y),
                ] {
                    input.pointer(&mut cx, phase, x, y).unwrap();
                }
                assert_eq!(
                    events.lock().unwrap().as_slice(),
                    &[ReorderListEvent::Reorder {
                        source: "row-4".into(),
                        before: Some("row-3".into()),
                    }]
                );
                events.lock().unwrap().clear();
                input
                    .pointer(&mut cx, nana_ui_input::PointerPhase::Down, x, source_y)
                    .unwrap();
                input
                    .pointer(&mut cx, nana_ui_input::PointerPhase::Move, x, target_y)
                    .unwrap();
                // An end point that does not map into the list cancels and still
                // releases the capture.
                cx.end_reorder_list_pointer(document, 1, f32::NAN, target_y, false)
                    .unwrap();
                assert_eq!(
                    events.lock().unwrap().as_slice(),
                    &[ReorderListEvent::Cancelled],
                    "live={live} control={control}"
                );
                assert_eq!(cx.world().pointer_capture(document, 1), None);
                assert!(!cx.read(list, ReorderList::is_dragging).unwrap());
            }
        }
    }

    #[test]
    fn escape_cancels_real_drag_releases_capture_and_keeps_pointer_up_from_reordering() {
        use nana_ui_input::{KeyInput, KeyState, PointerPhase};
        for from_button in [false, true] {
            let mut fixture = ButtonRows::new(false);
            let (x, y) = if from_button {
                fixture.on_button(0)
            } else {
                fixture.on_body(0)
            };
            let (_, end_y) = fixture.on_body(2);
            fixture.pointer(PointerPhase::Down, x, y);
            fixture.pointer(PointerPhase::Move, x, end_y + 20.0);
            assert!(fixture.dragging());
            assert!(fixture.captured());
            let result = fixture
                .input
                .press(
                    &mut fixture.context,
                    KeyInput::named("Escape", "Escape", KeyState::Pressed, Default::default()),
                    None,
                    None,
                )
                .unwrap();
            assert!(result.handled && result.prevent_default);
            assert!(!fixture.dragging());
            assert!(!fixture.captured());
            assert_eq!(fixture.painted_drag(), None);
            assert_eq!(fixture.take_events(), [ReorderListEvent::Cancelled]);
            fixture.pointer(PointerPhase::Move, x, end_y + 25.0);
            fixture.pointer(PointerPhase::Up, x, end_y + 25.0);
            assert_eq!(fixture.take_events(), Vec::<ReorderListEvent>::new());
            assert!(fixture.take_clicks().is_empty());
            // The next press starts an ordinary drag: the cancelled gesture
            // does not swallow it.
            fixture.pointer(PointerPhase::Down, x, y);
            fixture.pointer(PointerPhase::Move, x, end_y + 20.0);
            fixture.pointer(PointerPhase::Up, x, end_y + 20.0);
            assert!(matches!(
                fixture.take_events().as_slice(),
                [ReorderListEvent::Reorder { .. }]
            ));
        }
    }

    #[test]
    fn escape_release_and_repeat_leave_the_active_drag_until_the_first_press() {
        use nana_ui_input::{KeyInput, KeyState, PointerPhase};
        let mut fixture = ButtonRows::new(false);
        let (x, y) = fixture.on_body(0);
        let (_, end_y) = fixture.on_body(2);
        fixture.pointer(PointerPhase::Down, x, y);
        fixture.pointer(PointerPhase::Move, x, end_y);
        let release = KeyInput::named("Escape", "Escape", KeyState::Released, Default::default());
        fixture
            .input
            .press(&mut fixture.context, release, None, None)
            .unwrap();
        let mut repeat = KeyInput::named("Escape", "Escape", KeyState::Pressed, Default::default());
        repeat.repeat = true;
        fixture
            .input
            .press(&mut fixture.context, repeat, None, None)
            .unwrap();
        assert!(fixture.dragging() && fixture.captured());
        assert!(fixture.take_events().is_empty());
        fixture
            .input
            .press(
                &mut fixture.context,
                KeyInput::named("Escape", "Escape", KeyState::Pressed, Default::default()),
                None,
                None,
            )
            .unwrap();
        assert_eq!(fixture.take_events(), [ReorderListEvent::Cancelled]);
        fixture.pointer(PointerPhase::Up, x, end_y);
        assert!(fixture.take_events().is_empty());
    }

    /// Two live tree lists side by side in one document: the main list `a`
    /// (`a1` an effect, `a2` a mask, `a3` content) and the aside `b` (`b1` an
    /// effect that takes nothing inside, `b2` a group that takes effects
    /// inside); effects and masks may sit beside either row of `b` and at its
    /// top level. Each list's events are recorded.
    struct TwoLists {
        context: AppContext,
        lists: [crate::Entity<ReorderList>; 2],
        rows: [Vec<LayoutBox>; 2],
        events: [Arc<std::sync::Mutex<Vec<ReorderListEvent>>>; 2],
        input: crate::HeadlessInput,
    }

    impl TwoLists {
        /// `groups` are the two lists' transfer groups.
        fn new(groups: [Option<&str>; 2]) -> Self {
            Self::with(groups, true)
        }

        /// Flat lists without `tree_drop`.
        fn with(groups: [Option<&str>; 2], tree_drop: bool) -> Self {
            let document = document();
            let mut context = AppContext::new();
            let panel = context
                .create_component(
                    document,
                    crate::Stack::row(20.0).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(420.0));
                    }),
                )
                .unwrap();
            let beside = || ReorderAccepts::kinds(["effect", "mask"]);
            let items = [
                vec![
                    ReorderItem::new("a1", "a1").kind("effect"),
                    ReorderItem::new("a2", "a2").kind("mask"),
                    ReorderItem::new("a3", "a3").kind("content"),
                ],
                vec![
                    ReorderItem::new("b1", "b1")
                        .kind("effect")
                        .accepts_inside(ReorderAccepts::Nothing)
                        .accepts_sibling(beside()),
                    ReorderItem::new("b2", "b2")
                        .kind("group")
                        .accepts_inside(ReorderAccepts::kinds(["effect"]))
                        .accepts_sibling(beside()),
                ],
            ];
            let mut lists = Vec::new();
            let mut events = Vec::new();
            for (items, group) in items.into_iter().zip(groups) {
                let count = items.len();
                let mut list = ReorderList::new(items)
                    .live_rows(true)
                    .tree_drop(tree_drop)
                    .accepts_root(ReorderAccepts::kinds(["effect", "mask", "content"]))
                    .style(NodeStyle::default())
                    .with_width(200.0);
                if let Some(group) = group {
                    list = list.transfer_group(group);
                }
                let list = context.create_detached_component(document, list).unwrap();
                context.append_child(panel, list).unwrap();
                for _ in 0..count {
                    let row = context
                        .create_detached_component(
                            document,
                            crate::Stack::column(0.0).with_layout(|layout| {
                                layout.width = Some(LengthSpec::Fill);
                                layout.height = Some(LengthSpec::Px(28.0));
                            }),
                        )
                        .unwrap();
                    context.append_child(list, row).unwrap();
                }
                let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
                let observed = Arc::clone(&recorded);
                context
                    .on(list, move |_, event: &ReorderListEvent, _| {
                        observed.lock().unwrap().push(event.clone());
                    })
                    .unwrap();
                lists.push(list);
                events.push(recorded);
            }
            context
                .layout_document(document, crate::LayoutViewport::new(480.0, 300.0))
                .unwrap();
            context.rebuild_hit_test(document);
            let lists: [crate::Entity<ReorderList>; 2] = lists.try_into().unwrap();
            let rows = lists.map(|list| {
                let bounds = context
                    .world()
                    .component_layout_box(list.stable_id())
                    .unwrap();
                context.reorder_row_boxes(list.stable_id(), bounds)
            });
            assert!(
                rows[1][0].x > rows[0][0].x + rows[0][0].width,
                "the lists sit side by side"
            );
            let input = crate::HeadlessInput::bind(&mut context, document);
            Self {
                context,
                lists,
                rows,
                events: events.try_into().unwrap(),
                input,
            }
        }

        fn pointer(&mut self, phase: nana_ui_input::PointerPhase, (x, y): (f32, f32)) {
            self.input.pointer(&mut self.context, phase, x, y).unwrap();
        }

        /// The point `offset` of the way down row `row` of list `list`.
        fn at(&self, list: usize, row: usize, offset: f32) -> (f32, f32) {
            let row = self.rows[list][row];
            (row.x + 40.0, row.y + row.height * offset)
        }

        /// Presses row `row` of list `a` and moves it to `to`.
        fn drag(&mut self, row: usize, to: (f32, f32)) {
            use nana_ui_input::PointerPhase;
            let from = self.at(0, row, 0.5);
            self.pointer(PointerPhase::Down, from);
            self.pointer(PointerPhase::Move, (from.0, from.1 + 6.0));
            self.pointer(PointerPhase::Move, to);
        }

        fn visual(&self, list: usize) -> Option<ReorderDragVisual> {
            self.context
                .read(self.lists[list], ReorderList::drag_visual)
                .unwrap()
        }

        /// The drop list `list` marks now.
        fn mark(&self, list: usize) -> Option<ReorderDropMark> {
            let id = self.lists[list].stable_id();
            let bounds = self.context.world().component_layout_box(id).unwrap();
            self.visual(list)?.drop_mark(bounds, &self.rows[list])
        }

        fn take_events(&self, list: usize) -> Vec<ReorderListEvent> {
            std::mem::take(&mut *self.events[list].lock().unwrap())
        }

        fn dragging(&self) -> bool {
            self.context
                .read(self.lists[0], ReorderList::is_dragging)
                .unwrap()
        }

        fn captured(&self) -> bool {
            !self.context.world().pointer_captures(document()).is_empty()
        }
    }

    impl ReorderList {
        fn with_width(mut self, width: f32) -> Self {
            Arc::make_mut(&mut self.style.layout).width = Some(LengthSpec::Px(width));
            self
        }
    }

    #[test]
    fn a_row_dragged_into_a_row_of_another_list_of_its_group_drops_there() {
        use nana_ui_input::PointerPhase;
        let mut lists = TwoLists::new([Some("layers"), Some("layers")]);
        let into = lists.at(1, 1, 0.5);
        lists.drag(0, into);
        // The other list marks the row the effect goes inside; the dragged
        // row keeps its outline and its own list marks no drop.
        assert_eq!(
            lists.mark(1),
            Some(ReorderDropMark::Inside(tree_insert_line(
                lists.rows[1][1],
                TreeDropPosition::Inside,
                0.0
            )))
        );
        let own = lists.visual(0).expect("the drag goes on");
        assert_eq!(own.source, Some(0));
        assert!(own.carried_box(&lists.rows[0]).is_some());
        assert_eq!(lists.mark(0), None);
        let incoming = lists.visual(1).unwrap();
        assert_eq!((incoming.source, incoming.carried()), (None, None));
        assert_eq!(incoming.carried_box(&lists.rows[1]), None);
        assert!(lists.captured(), "the source list keeps the pointer");

        lists.pointer(PointerPhase::Up, into);
        assert_eq!(
            lists.take_events(1),
            [tree_drop("a1", "b2", TreeDropPosition::Inside)]
        );
        assert_eq!(lists.take_events(0), Vec::<ReorderListEvent>::new());
        assert!(!lists.dragging());
        assert!(!lists.captured());
        assert_eq!(lists.visual(0), None);
        assert_eq!(lists.visual(1), None);
    }

    #[test]
    fn beside_the_rows_of_another_list_its_sibling_rules_decide() {
        use nana_ui_input::PointerPhase;
        let mut lists = TwoLists::new([Some("layers"), Some("layers")]);
        // A mask before `b1`, which takes nothing inside: its top half.
        let before = lists.at(1, 0, 0.3);
        lists.drag(1, before);
        assert!(matches!(lists.mark(1), Some(ReorderDropMark::Line(_))));
        lists.pointer(PointerPhase::Up, before);
        assert_eq!(
            lists.take_events(1),
            [tree_drop("a2", "b1", TreeDropPosition::Before)]
        );
        // After `b2`, which takes effects but not masks inside.
        let after = lists.at(1, 1, 0.6);
        lists.drag(1, after);
        lists.pointer(PointerPhase::Up, after);
        assert_eq!(
            lists.take_events(1),
            [tree_drop("a2", "b2", TreeDropPosition::After)]
        );
        assert_eq!(lists.take_events(0), Vec::<ReorderListEvent>::new());
    }

    #[test]
    fn a_kind_the_other_list_refuses_is_marked_and_dropped_nowhere() {
        use nana_ui_input::PointerPhase;
        let mut lists = TwoLists::new([Some("layers"), Some("layers")]);
        // Content may sit beside neither row of `b` nor inside one.
        for offset in [0.1, 0.5, 0.9] {
            for row in 0..2 {
                let point = lists.at(1, row, offset);
                lists.drag(2, point);
                assert_eq!(lists.mark(1), None, "row {row} at {offset}");
                assert_eq!(lists.mark(0), None, "nor over its own list");
                lists.pointer(PointerPhase::Up, point);
                assert_eq!(lists.take_events(1), Vec::<ReorderListEvent>::new());
                assert_eq!(lists.take_events(0), Vec::<ReorderListEvent>::new());
                assert!(!lists.dragging() && !lists.captured());
            }
        }
    }

    #[test]
    fn lists_without_a_shared_group_keep_their_drags() {
        use nana_ui_input::PointerPhase;
        for groups in [
            [None, None],
            [Some("layers"), None],
            [None, Some("layers")],
            [Some("layers"), Some("other")],
        ] {
            let mut lists = TwoLists::new(groups);
            let into = lists.at(1, 1, 0.5);
            lists.drag(0, into);
            assert_eq!(lists.visual(1), None, "{groups:?}");
            assert!(lists.visual(0).is_some());
            lists.pointer(PointerPhase::Up, into);
            assert_eq!(lists.take_events(1), Vec::<ReorderListEvent>::new());
            assert_eq!(lists.take_events(0), Vec::<ReorderListEvent>::new());
        }
    }

    #[test]
    fn back_over_its_own_list_a_drag_is_its_own_again() {
        use nana_ui_input::PointerPhase;
        let mut lists = TwoLists::new([Some("layers"), Some("layers")]);
        lists.drag(0, lists.at(1, 1, 0.5));
        assert!(lists.mark(1).is_some());
        let home = lists.at(0, 2, 0.5);
        lists.pointer(PointerPhase::Move, home);
        assert_eq!(lists.visual(1), None, "the other list lets go");
        assert!(matches!(lists.mark(0), Some(ReorderDropMark::Inside(_))));
        lists.pointer(PointerPhase::Up, home);
        assert_eq!(
            lists.take_events(0),
            [tree_drop("a1", "a3", TreeDropPosition::Inside)]
        );
        assert_eq!(lists.take_events(1), Vec::<ReorderListEvent>::new());
    }

    #[test]
    fn escape_over_another_list_cancels_on_both() {
        use nana_ui_input::{KeyInput, KeyState, PointerPhase};
        let mut lists = TwoLists::new([Some("layers"), Some("layers")]);
        let into = lists.at(1, 1, 0.5);
        lists.drag(0, into);
        assert!(lists.mark(1).is_some());
        let result = lists
            .input
            .press(
                &mut lists.context,
                KeyInput::named("Escape", "Escape", KeyState::Pressed, Default::default()),
                None,
                None,
            )
            .unwrap();
        assert!(result.handled);
        assert!(!lists.dragging() && !lists.captured());
        assert_eq!(lists.visual(0), None);
        assert_eq!(lists.visual(1), None);
        assert_eq!(lists.take_events(0), [ReorderListEvent::Cancelled]);
        lists.pointer(PointerPhase::Move, (into.0, into.1 + 2.0));
        lists.pointer(PointerPhase::Up, (into.0, into.1 + 2.0));
        assert_eq!(lists.take_events(0), Vec::<ReorderListEvent>::new());
        assert_eq!(lists.take_events(1), Vec::<ReorderListEvent>::new());
    }

    #[test]
    fn a_flat_list_of_the_group_takes_a_row_as_a_reorder() {
        use nana_ui_input::PointerPhase;
        let mut lists = TwoLists::with([Some("layers"), Some("layers")], false);
        // Before `b2`, which takes masks beside it.
        let before = lists.at(1, 1, 0.3);
        lists.drag(1, before);
        assert_eq!(lists.visual(0).map(|visual| visual.preview), Some(true));
        assert_eq!(
            lists.visual(0).unwrap().shift_offsets(&lists.rows[0]),
            [0.0; 3],
            "its own rows make no room while it is over the other list"
        );
        let incoming = lists.visual(1).unwrap();
        assert!(
            !incoming.preview,
            "the other list marks the drop with a line"
        );
        assert!(matches!(lists.mark(1), Some(ReorderDropMark::Line(_))));
        lists.pointer(PointerPhase::Up, before);
        assert_eq!(
            lists.take_events(1),
            [ReorderListEvent::Reorder {
                source: Arc::from("a2"),
                before: Some(Arc::from("b2")),
            }]
        );
        assert_eq!(lists.take_events(0), Vec::<ReorderListEvent>::new());
        // Below the last row: the end of the other list.
        let end = lists.at(1, 1, 0.8);
        lists.drag(0, end);
        lists.pointer(PointerPhase::Up, end);
        assert_eq!(
            lists.take_events(1),
            [ReorderListEvent::Reorder {
                source: Arc::from("a1"),
                before: None,
            }]
        );
        // Content may not sit beside `b2`.
        let refused = lists.at(1, 1, 0.3);
        lists.drag(2, refused);
        assert_eq!(lists.mark(1), None);
        lists.pointer(PointerPhase::Up, refused);
        assert_eq!(lists.take_events(1), Vec::<ReorderListEvent>::new());
        assert_eq!(lists.take_events(0), Vec::<ReorderListEvent>::new());
    }
}
