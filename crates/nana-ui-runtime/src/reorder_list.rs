//! Vertical list with thresholded reorder and optional tree-drop intents.
//!
//! Application code owns item identities, order, grouping, and persistence.
//! This type reports select, before-value reorder, and tree-drop results from
//! pointer geometry only.

use std::sync::Arc;

#[cfg(test)]
use nana_ui_core::UI_METRICS;
use nana_ui_core::{ControlSize, FlexDirection, LengthSpec, reorder_changes_position};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, LayoutBox,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, StandardVisual, UiWorld,
};

const DRAG_THRESHOLD: f32 = nana_ui_core::space::XS;
const DEFAULT_SPACING: f32 = nana_ui_core::space::XXS;
const INSERT_INSET: f32 = nana_ui_core::space::XS;
const INSERT_THICKNESS: f32 = nana_ui_core::space::XXS;

/// Placement resolved for a tree drop target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeDropPosition {
    Before,
    Inside,
    After,
}

/// Framework-owned tree drop intent. Applications keep node semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeDropIntent {
    pub target: Arc<str>,
    pub position: TreeDropPosition,
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

/// One row identity. A press on a control inside a live row (a focusable
/// node below the row, such as a button) is the control's click unless the
/// pointer moves past the drag threshold, when the row drags instead.
/// Optional [`Self::tools`] is a live child that keeps the pointer to itself:
/// hits there never start a drag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReorderItem {
    pub value: Arc<str>,
    pub label: Arc<str>,
    pub draggable: bool,
    pub drop_target: bool,
    pub selected: bool,
    pub disabled: bool,
    /// With [`ReorderList::tree_drop`], a drop on the row's middle goes
    /// inside it. A row that takes nothing inside splits into before and
    /// after halves instead. On by default.
    pub nest: bool,
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
            nest: true,
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

    /// Whether a tree drop can go inside this row; see [`Self::nest`].
    pub fn nest(mut self, nest: bool) -> Self {
        self.nest = nest;
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
    pub drop_targets: Arc<[bool]>,
    /// Which rows a tree drop can go inside ([`ReorderItem::nest`]).
    pub nest_targets: Arc<[bool]>,
    pub tree_drop: bool,
}

/// Where a drop would land, as drawn: a line between rows, or the row a
/// tree drop goes inside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReorderDropMark {
    Line(LayoutBox),
    Inside(LayoutBox),
}

impl ReorderDragVisual {
    /// The drop indicator over `rows` inside `bounds`: the insert line, or
    /// the target's line for a tree drop; `None` where a drop changes nothing.
    pub fn insert_line(&self, bounds: LayoutBox, rows: &[LayoutBox]) -> Option<LayoutBox> {
        self.drop_mark(bounds, rows).map(|mark| match mark {
            ReorderDropMark::Line(line) | ReorderDropMark::Inside(line) => line,
        })
    }

    /// [`Self::insert_line`], telling a line from a row a drop goes inside.
    pub fn drop_mark(&self, bounds: LayoutBox, rows: &[LayoutBox]) -> Option<ReorderDropMark> {
        if self.tree_drop {
            let (target, position) = tree_drop_target(
                rows,
                &self.drop_targets,
                &self.nest_targets,
                Some(self.source),
                self.x,
                self.y,
            )?;
            let mark = tree_insert_line(rows[target], position);
            return Some(if position == TreeDropPosition::Inside {
                ReorderDropMark::Inside(mark)
            } else {
                ReorderDropMark::Line(mark)
            });
        }
        let before = drop_before_index(rows, &self.drop_targets, Some(self.source), self.y);
        if !reorder_changes_position(rows.len(), self.source, before) {
            return None;
        }
        Some(ReorderDropMark::Line(reorder_insert_line(
            bounds, rows, before,
        )))
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
    /// Declared at construction: live row children own painting and the list
    /// body follows their layout. Never inferred from the tree at project time.
    pub live_rows: bool,
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
            live_rows: false,
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

    /// Declare that the host attaches live row children which own painting.
    /// Retained `items` stay the drag/hit-test model and are never painted as
    /// self-drawn rows in this mode.
    pub fn live_rows(mut self, live_rows: bool) -> Self {
        self.live_rows = live_rows;
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
            drop_targets: self.drop_target_flags().into(),
            nest_targets: self.nest_flags().into(),
            tree_drop: self.tree_drop,
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
        let drop_targets = self.drop_target_flags();
        if self.tree_drop {
            let (target, position) = tree_drop_target(
                rows,
                &drop_targets,
                &self.nest_flags(),
                Some(source),
                drag.x,
                drag.y,
            )?;
            let target = Arc::clone(&self.items[target].value);
            return Some(ReorderListEvent::TreeDrop {
                source: drag.source,
                intent: TreeDropIntent { target, position },
            });
        }
        let before = drop_before_index(rows, &drop_targets, Some(source), drag.y);
        if !reorder_changes_position(self.items.len(), source, before) {
            return None;
        }
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

    fn drop_target_flags(&self) -> Vec<bool> {
        self.items.iter().map(ReorderItem::is_drop_target).collect()
    }

    fn nest_flags(&self) -> Vec<bool> {
        self.items.iter().map(|item| item.nest).collect()
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
        let rows = self.reorder_row_boxes(id, bounds);
        let exclude = self.reorder_tool_boxes(entity);
        if exclude.iter().any(|reserved| reserved.contains(x, y)) {
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
        let Some(index) = item_at(&rows, &enabled, x, y) else {
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

fn drop_before_index(
    bounds: &[LayoutBox],
    drop_targets: &[bool],
    excluded: Option<usize>,
    y: f32,
) -> Option<usize> {
    bounds.iter().enumerate().find_map(|(index, row)| {
        (Some(index) != excluded
            && drop_targets.get(index).copied().unwrap_or(false)
            && y < row.y + row.height * 0.5)
            .then_some(index)
    })
}

fn tree_drop_target(
    bounds: &[LayoutBox],
    drop_targets: &[bool],
    nest_targets: &[bool],
    excluded: Option<usize>,
    x: f32,
    y: f32,
) -> Option<(usize, TreeDropPosition)> {
    bounds
        .iter()
        .enumerate()
        .find(|(index, row)| {
            Some(*index) != excluded
                && drop_targets.get(*index).copied().unwrap_or(false)
                && row.contains(x, y)
        })
        .map(|(index, row)| {
            let offset = (y - row.y) / row.height.max(1.0);
            let nest = nest_targets.get(index).copied().unwrap_or(true);
            let position = if !nest {
                if offset < 0.5 {
                    TreeDropPosition::Before
                } else {
                    TreeDropPosition::After
                }
            } else if offset < 0.25 {
                TreeDropPosition::Before
            } else if offset > 0.75 {
                TreeDropPosition::After
            } else {
                TreeDropPosition::Inside
            };
            (index, position)
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

fn tree_insert_line(target: LayoutBox, position: TreeDropPosition) -> LayoutBox {
    match position {
        TreeDropPosition::Before => LayoutBox {
            x: target.x + INSERT_INSET,
            y: target.y - 1.0,
            width: (target.width - INSERT_INSET * 2.0).max(0.0),
            height: INSERT_THICKNESS,
        },
        TreeDropPosition::After => LayoutBox {
            x: target.x + INSERT_INSET,
            y: target.y + target.height - 1.0,
            width: (target.width - INSERT_INSET * 2.0).max(0.0),
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
                ReorderItem::new("b", "Beta").nest(false),
                ReorderItem::new("c", "Gamma"),
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
            apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 12.0 });
            apply(&mut list, ReorderListPointer::Move { x: 40.0, y });
            assert_eq!(
                apply(&mut list, ReorderListPointer::Up { x: 40.0, y }),
                Some(ReorderListEvent::TreeDrop {
                    source: Arc::from("a"),
                    intent: TreeDropIntent {
                        target: Arc::from("b"),
                        position,
                    },
                }),
                "a drop at {offset} of the row"
            );
        }
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
    fn a_moved_drag_over_live_rows_outlines_its_row_and_shows_where_it_lands() {
        let mut context = AppContext::new();
        let document = document();
        let list = context
            .create_component(document, sample().live_rows(true).spacing(4.0))
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
        // Past the middle of the last row: it goes to the end.
        let end = boxes[2].y + 30.0;
        context
            .update_reorder_list_pointer(document, 1, x, end)
            .unwrap();
        let (insert, dragged) = geometry(&context);
        assert_eq!(dragged, Some(boxes[0]));
        let insert = insert.expect("an insert line");
        assert!(
            insert.y >= boxes[2].y + boxes[2].height,
            "the line {insert:?} sits under the last live row {:?}",
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
}
