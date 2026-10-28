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
use nana_ui_core::{ControlSize, FlexDirection, LengthSpec};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, LayoutBox,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, StandardVisual, UiWorld,
};

const DRAG_THRESHOLD: f32 = nana_ui_core::space::XS;
const DEFAULT_SPACING: f32 = nana_ui_core::space::XXS;
const INSERT_INSET: f32 = nana_ui_core::space::XS;
const INSERT_THICKNESS: f32 = nana_ui_core::space::XXS;
const DEFAULT_INDENT: f32 = nana_ui_core::space::LG;

include!("reorder_list_gap.rs");

pub use crate::tree_view::{TreeDropIntent, TreeDropPosition};

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
    pub tools: Option<StableNodeId>,
}

impl ReorderItem {
    pub fn new(value: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            draggable: true,
            drop_target: true,
            selected: false,
            disabled: false,
            depth: 0,
            kind: None,
            accepts_inside: ReorderAccepts::All,
            accepts_sibling: ReorderAccepts::All,
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
}

/// What a list shows while a row is dragged past the threshold: which row,
/// where the pointer is and which rows take a drop. The row boxes come from
/// layout — the list's own uniform rows, or its live row children — so the
/// drop indicator is resolved where those boxes are known.
#[derive(Debug, Clone, PartialEq)]
pub struct ReorderDragVisual {
    /// The dragged row's index in [`ReorderList::items`].
    pub source: usize,
    pub x: f32,
    pub y: f32,
    /// 拖动源相对抓取位置的呈现偏移，布局与落点继续使用稳定坐标。
    pub preview_offset: Option<[f32; 2]>,
    /// 活行普通排序的让位间距，呈现动画由调用方驱动。
    pub gap_spacing: Option<f32>,
    pub tree_drop: bool,
    /// Live rows in a flat drag: the carried rows follow the pointer and the
    /// rows between make room where they would land, both on the
    /// presentation layer over unchanged layout. The list draws no outline or
    /// insert line then; a tree drop keeps them.
    pub preview: bool,
    rules: Arc<DropRules>,
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
    pub fn carried(&self) -> std::ops::RangeInclusive<usize> {
        self.source..=self.rules.subtree_end(self.source)
    }

    /// The box around the carried rows, among `rows`.
    pub fn carried_box(&self, rows: &[LayoutBox]) -> Option<LayoutBox> {
        let first = rows.get(self.source)?;
        let last = rows.get(*self.carried().end()).unwrap_or(first);
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

    /// [`Self::insert_line`], telling a line from a row a drop goes inside.
    /// A tree drop's line is indented to the depth the row lands at.
    pub fn drop_mark(&self, bounds: LayoutBox, rows: &[LayoutBox]) -> Option<ReorderDropMark> {
        if let Some(spacing) = self.gap_spacing {
            return self.gap_drop_mark(bounds, rows, spacing);
        }
        if self.tree_drop {
            let drop = self.rules.tree_drop(rows, self.source, self.x, self.y)?;
            let indent = f32::from(self.rules.rows[drop.target].depth) * self.rules.indent;
            let mark = tree_insert_line(rows[drop.anchor], drop.position, indent);
            return Some(if drop.position == TreeDropPosition::Inside {
                ReorderDropMark::Inside(mark)
            } else {
                ReorderDropMark::Line(mark)
            });
        }
        let before = self.rules.flat_drop(rows, self.source, self.y)?;
        Some(ReorderDropMark::Line(reorder_insert_line(
            bounds, rows, before,
        )))
    }

    /// How far the carried rows have been moved from where they were
    /// pressed, `[x, y]`.
    pub fn preview_offset(&self) -> [f32; 2] {
        self.preview_offset.unwrap_or([0.0, 0.0])
    }

    /// How far each of `rows` moves down (up when negative) to make room for
    /// the carried rows where they would land: the rows between their place
    /// and that spot shift by the carried block's height, its spacing
    /// included. All zero where the drop changes nothing or may not land,
    /// and for the carried rows themselves, which follow the pointer.
    pub fn shift_offsets(&self, rows: &[LayoutBox]) -> Vec<f32> {
        let mut offsets = vec![0.0; rows.len()];
        let carried = self.carried();
        let (start, end) = (*carried.start(), *carried.end());
        if rows.len() != self.rows() || end >= rows.len() {
            return offsets;
        }
        let Some(before) = self.rules.flat_drop(rows, self.source, self.y) else {
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
}

/// A list's rows as a flattened tree, and where a dragged subtree may land.
#[derive(Debug, Clone, PartialEq)]
struct DropRules {
    rows: Vec<DropRow>,
    root: ReorderAccepts,
    indent: f32,
}

/// A resolved tree drop: `position` of `target`, drawn against `anchor`
/// (the row under the pointer for a line after a deeper row).
#[derive(Debug, Clone, Copy, PartialEq)]
struct TreeDrop {
    target: usize,
    position: TreeDropPosition,
    anchor: usize,
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
                })
                .collect(),
            root: list.accepts_root.clone(),
            indent: list.indent,
        }
    }

    fn depth(&self, index: usize) -> u16 {
        self.rows[index].depth
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
    fn allows(&self, source: usize, target: usize, position: TreeDropPosition) -> bool {
        if (source..=self.subtree_end(source)).contains(&target) || !self.rows[target].target {
            return false;
        }
        let kind = self.rows[source].kind.as_deref();
        match position {
            TreeDropPosition::Inside => self.rows[target].inside.accepts(kind),
            TreeDropPosition::Before | TreeDropPosition::After => {
                self.rows[target].sibling.accepts(kind)
                    && self.takes_child(self.parent(target), kind)
            }
        }
    }

    /// Whether the move puts `source` anywhere new.
    fn changes(&self, source: usize, target: usize, position: TreeDropPosition) -> bool {
        match position {
            TreeDropPosition::Before => self.next_sibling(source) != Some(target),
            TreeDropPosition::After => self.previous_sibling(source) != Some(target),
            TreeDropPosition::Inside => true,
        }
    }

    /// The first of `places` that `source` may go to, if it moves it.
    fn first_allowed(
        &self,
        source: usize,
        places: impl IntoIterator<Item = TreeDrop>,
    ) -> Option<TreeDrop> {
        places
            .into_iter()
            .find(|drop| self.allows(source, drop.target, drop.position))
            .filter(|drop| self.changes(source, drop.target, drop.position))
    }

    /// Where a tree drop of `source` at `(x, y)` over `boxes` lands. The top
    /// quarter of a row is before it, the middle inside (halves without
    /// inside, for a row that does not take the source's kind), the bottom
    /// after it — or, over a row whose children show below it, before its
    /// first child. After the last row of nested rows, `x` picks the depth:
    /// each [`ReorderList::indent`] left of the row's own is one level out.
    fn tree_drop(&self, boxes: &[LayoutBox], source: usize, x: f32, y: f32) -> Option<TreeDrop> {
        let hovered = boxes
            .iter()
            .take(self.rows.len())
            .position(|row| row.contains(x, y))?;
        let end = self.subtree_end(source);
        if (source..=end).contains(&hovered) {
            return None;
        }
        let row = boxes[hovered];
        let offset = (y - row.y) / row.height.max(1.0);
        let inside = self.rows[hovered]
            .inside
            .accepts(self.rows[source].kind.as_deref());
        let at = |target, position, anchor| TreeDrop {
            target,
            position,
            anchor,
        };
        if offset < if inside { 0.25 } else { 0.5 } {
            return self.first_allowed(source, [at(hovered, TreeDropPosition::Before, hovered)]);
        }
        if inside && offset <= 0.75 {
            return self.first_allowed(source, [at(hovered, TreeDropPosition::Inside, hovered)]);
        }
        // The row after, past what the drag carries.
        let next = if hovered + 1 == source {
            end + 1
        } else {
            hovered + 1
        };
        let depth = self.depth(hovered);
        if next < self.rows.len() && self.depth(next) > depth {
            return self.first_allowed(
                source,
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
            source,
            (floor..=wanted).rev().filter_map(|level| {
                self.ancestor_at(hovered, level)
                    .map(|target| at(target, TreeDropPosition::After, hovered))
            }),
        )
    }

    /// The row a flat drop of `source` at `y` goes before (`None` for the
    /// end); `None` overall where it changes nothing or may not land.
    fn flat_drop(&self, boxes: &[LayoutBox], source: usize, y: f32) -> Option<Option<usize>> {
        let end = self.subtree_end(source);
        let before = boxes
            .iter()
            .take(self.rows.len())
            .enumerate()
            .find_map(|(index, row)| {
                (!(source..=end).contains(&index)
                    && self.rows[index].target
                    && y < row.y + row.height * 0.5)
                    .then_some(index)
            });
        let unchanged = match before {
            Some(before) => before == end + 1,
            None => end + 1 == self.rows.len(),
        };
        let kind = self.rows[source].kind.as_deref();
        let accepted = before.is_none_or(|before| self.rows[before].sibling.accepts(kind));
        (!unchanged && accepted).then_some(before)
    }

    /// Where a keyboard `step` takes `source`.
    fn key_drop(&self, source: usize, step: ReorderKeyMove) -> Option<TreeDrop> {
        let at = |target, position| TreeDrop {
            target,
            position,
            anchor: target,
        };
        match step {
            ReorderKeyMove::Up => self.first_allowed(
                source,
                [at(self.previous_sibling(source)?, TreeDropPosition::Before)],
            ),
            ReorderKeyMove::Down => self.first_allowed(
                source,
                [at(self.next_sibling(source)?, TreeDropPosition::After)],
            ),
            ReorderKeyMove::Indent => {
                let parent = self.previous_sibling(source)?;
                let child = self.depth(parent) + 1;
                let last_child = (parent + 1..source)
                    .rev()
                    .find(|&row| self.depth(row) == child);
                self.first_allowed(
                    source,
                    last_child
                        .map(|row| at(row, TreeDropPosition::After))
                        .into_iter()
                        .chain([at(parent, TreeDropPosition::Inside)]),
                )
            }
            ReorderKeyMove::Outdent => {
                self.first_allowed(source, [at(self.parent(source)?, TreeDropPosition::After)])
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

/// Vertical list. NanaUI does not mutate item order on reorder or tree drop.
#[derive(Debug, Clone, PartialEq)]
pub struct ReorderList {
    pub items: Vec<ReorderItem>,
    pub spacing: f32,
    pub size: ControlSize,
    pub tree_drop: bool,
    /// How far each level of [`ReorderItem::depth`] sits in, for the insert
    /// line and for picking a depth with the pointer.
    pub indent: f32,
    /// Which rows a tree drop may put at the top level. All by default.
    pub accepts_root: ReorderAccepts,
    /// Declared at construction: live row children own painting and the list
    /// body follows their layout. Never inferred from the tree at project time.
    pub live_rows: bool,
    /// 用原行子树投影65%透明的拖动预览，保留稳定实体与编辑历史。
    pub drag_preview: bool,
    /// 提供稳定让位几何，由应用 Motion 轨道呈现相邻行移动。
    pub drag_gap: bool,
    pub label: Option<Arc<str>>,
    pub style: NodeStyle,
    drag: Option<ReorderDrag>,
    armed: Option<ArmedPress>,
}

impl ReorderList {
    pub fn new(items: impl IntoIterator<Item = ReorderItem>) -> Self {
        Self {
            items: items.into_iter().collect(),
            spacing: DEFAULT_SPACING,
            size: ControlSize::Small,
            tree_drop: false,
            indent: DEFAULT_INDENT,
            accepts_root: ReorderAccepts::All,
            live_rows: false,
            drag_preview: false,
            drag_gap: false,
            label: None,
            style: NodeStyle::default(),
            drag: None,
            armed: None,
        }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing.max(0.0);
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

    /// 启用随抓取点移动的半透明预览，默认关闭。
    pub fn drag_preview(mut self, enabled: bool) -> Self {
        self.drag_preview = enabled;
        self
    }

    /// 提供实时让位目标，默认关闭；应用持有呈现动画。
    pub fn drag_gap(mut self, enabled: bool) -> Self {
        self.drag_gap = enabled;
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
    /// threshold, gone again when the gesture ends.
    pub fn drag_visual(&self) -> Option<ReorderDragVisual> {
        let drag = self.drag.as_ref().filter(|drag| drag.moved)?;
        let source = self.item_index(&drag.source)?;
        if !self.items[source].is_source() {
            return None;
        }
        Some(ReorderDragVisual {
            source,
            x: drag.x,
            y: drag.y,
            preview_offset: self
                .drag_preview
                .then_some([drag.x - drag.start_x, drag.y - drag.start_y]),
            gap_spacing: (self.drag_gap && self.live_rows && self.drag_preview && !self.tree_drop)
                .then_some(self.spacing.max(0.0)),
            tree_drop: self.tree_drop,
            preview: self.live_rows && !self.tree_drop,
            rules: Arc::new(DropRules::of(self)),
        })
    }

    /// The event a keyboard `step` of row `source` publishes: a
    /// [`ReorderListEvent::TreeDrop`] with [`Self::tree_drop`], else a
    /// [`ReorderListEvent::Reorder`] (only [`ReorderKeyMove::Up`] and
    /// [`ReorderKeyMove::Down`]). `None` where the row cannot go, or would
    /// not move. The row carries the rows it holds.
    pub fn key_move(&self, source: &str, step: ReorderKeyMove) -> Option<ReorderListEvent> {
        let index = self.item_index(source)?;
        if self.drag.is_some() || !self.items[index].is_source() {
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
        });
        None
    }

    fn move_to(&mut self, x: f32, y: f32) {
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
        let rules = DropRules::of(self);
        if self.tree_drop {
            let drop = rules.tree_drop(rows, source, drag.x, drag.y)?;
            let target = Arc::clone(&self.items[drop.target].value);
            return Some(ReorderListEvent::TreeDrop {
                source: drag.source,
                intent: TreeDropIntent {
                    target,
                    position: drop.position,
                },
            });
        }
        let before = rules.flat_drop(rows, source, drag.y)?;
        let before = before.map(|index| Arc::clone(&self.items[index].value));
        Some(ReorderListEvent::Reorder {
            source: drag.source,
            before,
        })
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
        let visual = StandardVisual::ReorderList {
            rows: if live_rows {
                Arc::<[ReorderRowPaint]>::from([])
            } else {
                self.paint_rows()
            },
            size: self.size,
            spacing: self.spacing,
            preview_enabled: self.drag_preview,
            drag: self.drag_visual(),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        if layout.width.is_none() {
            layout.width = Some(LengthSpec::Fill);
        }
        if live_rows {
            layout.direction = Some(FlexDirection::Column);
            if layout.gap.is_none() {
                layout.gap = Some(LengthSpec::Px(self.spacing.max(0.0)));
            }
            if layout.height.is_none() {
                layout.height = Some(LengthSpec::Shrink);
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
        // 输入点进入列表的布局坐标系，与保留的行盒子一致，覆盖祖先滚动与呈现变换。
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
        let Some((x, y)) = self.world().pointer_layout_position(target, x, y) else {
            return Ok(false);
        };
        let rows = self.reorder_row_boxes(target, bounds);
        self.update_component(entity, |list, _| {
            list.apply_pointer_with_rows(ReorderListPointer::Move { x, y }, &rows, &[]);
            list.is_dragging()
        })
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
        // 取消始终释放捕获；结束点使用与按下和移动相同的布局坐标。
        let point = self.world().pointer_layout_position(target, x, y);
        let cancel = cancel || point.is_none();
        let (x, y) = point.unwrap_or((x, y));
        let bounds = self.world().component_layout_box(target);
        let rows = bounds
            .map(|bounds| self.reorder_row_boxes(target, bounds))
            .unwrap_or_default();
        self.update_component(entity, |list, cx| {
            let pointer = if cancel {
                ReorderListPointer::Cancel
            } else {
                ReorderListPointer::Up { x, y }
            };
            if let Some(event) = list.apply_pointer_with_rows(pointer, &rows, &[]) {
                cx.emit(event);
            }
            cx.mutations().release_pointer(pointer_id, target);
            true
        })
    }

    /// Escape 终止文档中捕获指针的列表手势，并清理按下记录，保证松手保持取消结果。
    pub fn cancel_reorder_list_drag(
        &mut self,
        document: crate::DocumentId,
    ) -> Result<bool, crate::FrameworkError> {
        let captures = self.world().pointer_captures(document);
        let mut cancelled = false;
        for (pointer_id, target) in captures {
            if self.is_reorder_list(target)
                && self.end_reorder_list_pointer(document, pointer_id, 0.0, 0.0, true)?
            {
                // 与指针 Cancel 共用终止路径，Up 随后只完成原生按钮释放。
                self.release_pointer(document, pointer_id);
                cancelled = true;
            }
        }
        Ok(cancelled)
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
        let Some((x, y)) = self.world().pointer_layout_position(list_id, x, y) else {
            return Ok(false);
        };
        let started = self.update_component(entity, |list, cx| {
            let Some(armed) = list.armed.as_mut() else {
                return false;
            };
            armed.drag.follow(x, y);
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

#[cfg(test)]
#[path = "../tests/fixtures/reorder_pointer_coordinates.rs"]
mod pointer_coordinates_tests;

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
    include!("reorder_list_escape_tests.rs");
    include!("reorder_list_gap_tests.rs");
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
            assert_eq!(visual.carried(), 0..=3);
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
        assert_eq!(visual.unwrap().carried(), 2..=3);
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
            context.world().standard_visual(id),
            Some(StandardVisual::ReorderList {
                rows: sample().paint_rows(),
                size: ControlSize::Small,
                spacing: DEFAULT_SPACING,
                preview_enabled: false,
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
        let geometry =
            |context: &AppContext| match context.world().component_geometry(list.stable_id()) {
                Some(crate::ComponentGeometry::ReorderList {
                    insert, dragged, ..
                }) => (insert.map(|(line, _)| line), dragged.map(|(row, _)| row)),
                other => panic!("not reorder list geometry: {other:?}"),
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
            context.world().standard_visual(id),
            Some(StandardVisual::ReorderList {
                rows: Arc::from([]),
                size: ControlSize::Small,
                spacing: DEFAULT_SPACING,
                preview_enabled: false,
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
                .map(|visual| visual.source)
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
                    cx.read(list, |v| v.drag_visual().map(|v| v.source))
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
}
