//! Selection requests and keyboard navigation for [`crate::List`] and
//! [`crate::view::each_virtual`].
//!
//! Like [`crate::Table`] and [`crate::TreeView`], a `List` owns neither its
//! data nor its selection: it emits [`ListSelectionRequested`] when an item
//! is pressed or a selection key is pressed while focus is inside the list, and
//! the application applies the gesture to its [`nana_ui_core::SelectionState`].
//!
//! Only lists that opt in — by setting [`crate::List::multiselectable`] or
//! registering a [`ListSelectionRequested`] listener — handle selection
//! presses and keys, leaving plain lists and [`crate::RovingFocusGroup`]
//! navigation untouched.

use nana_ui_core::{SelectionGesture, SelectionModifiers, SelectionStep};
use nana_ui_input::InputModifiers;

use crate::selection_modifiers;
use crate::{
    AppContext, DocumentId, Entity, FrameworkError, InteractiveCard, List, ListItem, NodePointer,
    NodePointerHooks, StableNodeId,
};

/// Items a Page Up / Page Down steps by in a plain [`List`], matching the
/// data table's page step.
const LIST_PAGE_ITEMS: usize = 10;

/// A press on a list item or a selection key while focus is inside a
/// selection-active [`List`] asked for the selection to change.
///
/// For keyboard navigation (`ArrowUp` / `ArrowDown` / `Home` / `End` /
/// `PageUp` / `PageDown`, with or without `Shift` / `Ctrl`), [`Self::step`]
/// carries the [`SelectionStep`] so virtualized lists can navigate to offscreen
/// keys via [`nana_ui_core::SelectionState::navigate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListSelectionRequested {
    pub item: StableNodeId,
    pub gesture: SelectionGesture,
    pub step: Option<SelectionStep>,
}

pub(crate) static LIST_CAPABILITIES: crate::framework::ListCapabilities =
    crate::framework::ListCapabilities {
        selection_key: AppContext::list_selection_key,
        pointer_press: AppContext::list_pointer_press,
    };

pub(crate) const LIST_ITEM_POINTER: NodePointerHooks<ListItem> = NodePointerHooks {
    moved: |_, _, _| Ok(false),
    left: |_, _| Ok(()),
    pressed: list_item_pressed,
    released: |_, _, _| Ok(false),
    wheel: |_, _, _| Ok(false),
};

fn list_item_pressed(
    cx: &mut AppContext,
    item: Entity<ListItem>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if pointer.button != 0 {
        return Ok(false);
    }
    if !cx.read(item, |i| i.interactive())? {
        return Ok(false);
    }
    let Some(list) = enclosing_selection_list(cx, item.stable_id()) else {
        return Ok(false);
    };
    let gesture = SelectionGesture::Press(selection_modifiers(pointer.modifiers));
    cx.update_component(list, |_, view| {
        view.emit(ListSelectionRequested {
            item: item.stable_id(),
            gesture,
            step: None,
        });
    })?;
    // Keep normal focus + Activate delivery intact.
    Ok(false)
}

fn is_selection_active_list(cx: &AppContext, list: Entity<List>) -> bool {
    cx.read(list, |l| l.multiselectable).is_ok_and(|multi| {
        multi || cx.has_event_listener::<ListSelectionRequested>(list.stable_id())
    })
}

/// Nearest enclosing [`List`] above `node` that opted into selection. Stops
/// if an enclosing `Table` or `TreeView` is met first.
fn enclosing_selection_list(cx: &AppContext, node: StableNodeId) -> Option<Entity<List>> {
    let mut current = cx.world().parent_id(node);
    while let Some(id) = current {
        if let Some(list) = cx.view_entity::<List>(id) {
            return is_selection_active_list(cx, list).then_some(list);
        }
        if cx.view_entity::<crate::Table>(id).is_some()
            || cx.view_entity::<crate::TreeView>(id).is_some()
        {
            return None;
        }
        current = cx.world().parent_id(id);
    }
    None
}

fn is_selectable_node_marker(cx: &AppContext, id: StableNodeId) -> bool {
    cx.view_entity::<ListItem>(id).is_some()
        || cx.view_entity::<InteractiveCard>(id).is_some()
        || cx.world().selected_override(id).is_some()
}

/// Find the selection-active [`List`] enclosing `target` and the item node
/// representing the row/card under it, or `None` when `target` is a nested
/// interactive control inside another selectable item.
fn enclosing_list_and_item(
    cx: &AppContext,
    target: StableNodeId,
    from_pointer: bool,
) -> Option<(Entity<List>, StableNodeId)> {
    let mut marker = is_selectable_node_marker(cx, target).then_some(target);
    let mut last_under_list = target;
    let mut current = cx.world().parent_id(target);
    while let Some(id) = current {
        if let Some(list) = cx.view_entity::<List>(id) {
            if !is_selection_active_list(cx, list) {
                return None;
            }
            if from_pointer && marker.is_some_and(|m| m != target) {
                // Pointer landed on a nested control (e.g. a delete button)
                // inside an already-selectable item: leave row selection alone.
                return None;
            }
            let item = marker.unwrap_or_else(|| {
                if cx.world().is_virtual_list(list.stable_id()) {
                    target
                } else {
                    last_under_list
                }
            });
            return Some((list, item));
        }
        if cx.view_entity::<crate::Table>(id).is_some()
            || cx.view_entity::<crate::TreeView>(id).is_some()
        {
            return None;
        }
        if marker.is_none() && is_selectable_node_marker(cx, id) {
            marker = Some(id);
        }
        last_under_list = id;
        current = cx.world().parent_id(id);
    }
    None
}

fn is_list_item_enabled(cx: &AppContext, document: DocumentId, id: StableNodeId) -> bool {
    let Some(node) = cx.world().node(id) else {
        return false;
    };
    if node.document != document
        || !cx.world().is_mounted(id)
        || cx.world().motion_blocks_input(id)
        || !cx.world().is_overlay_reachable(id)
    {
        return false;
    }
    if cx
        .world()
        .accessibility(id)
        .is_some_and(|state| state.disabled || state.display_only)
    {
        return false;
    }
    cx.world()
        .interaction(id)
        .is_some_and(|interaction| interaction.focusable)
        && cx.sequential_focus_candidate(document, id)
}

/// Mounted focusable item roots under a plain [`List`], in document order.
fn mounted_list_items(
    cx: &AppContext,
    document: DocumentId,
    list_id: StableNodeId,
) -> Vec<StableNodeId> {
    let mut items = Vec::new();
    let Some(root) = cx.world().node(list_id) else {
        return items;
    };
    let mut stack: Vec<StableNodeId> = root.children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if is_list_item_enabled(cx, document, id) {
            items.push(id);
            continue;
        }
        if is_selectable_node_marker(cx, id) {
            // A disabled selectable item stays unselectable and is skipped.
            continue;
        }
        if let Some(node) = cx.world().node(id) {
            stack.extend(node.children.iter().rev().copied());
        }
    }
    items
}

fn step_in_mounted(
    items: &[StableNodeId],
    from: Option<usize>,
    step: SelectionStep,
) -> Option<usize> {
    let len = items.len();
    if len == 0 {
        return None;
    }
    let target = match (step, from) {
        (SelectionStep::First, _) => Some(0),
        (SelectionStep::Last, _) => Some(len - 1),
        (SelectionStep::Next | SelectionStep::Forward(_), None) => Some(0),
        (SelectionStep::Previous | SelectionStep::Backward(_), None) => Some(len - 1),
        (SelectionStep::Next, Some(from)) => (from + 1 < len).then_some(from + 1),
        (SelectionStep::Previous, Some(from)) => from.checked_sub(1),
        (SelectionStep::Forward(count), Some(from)) => {
            let next = from.saturating_add(count.max(1)).min(len - 1);
            (next > from).then_some(next)
        }
        (SelectionStep::Backward(count), Some(from)) => {
            let prev = from.saturating_sub(count.max(1));
            (prev < from).then_some(prev)
        }
    }?;
    (from != Some(target)).then_some(target)
}

impl AppContext {
    pub(crate) fn list_pointer_press(
        &mut self,
        target: StableNodeId,
        modifiers: InputModifiers,
    ) -> Result<bool, FrameworkError> {
        if self.view_entity::<ListItem>(target).is_some() {
            return Ok(false);
        }
        let Some((list, item)) = enclosing_list_and_item(self, target, true) else {
            return Ok(false);
        };
        if self
            .world()
            .accessibility(item)
            .is_some_and(|state| state.disabled || state.display_only)
        {
            return Ok(false);
        }
        let gesture = SelectionGesture::Press(selection_modifiers(modifiers));
        self.update_component(list, |_, view| {
            view.emit(ListSelectionRequested {
                item,
                gesture,
                step: None,
            });
        })?;
        Ok(false)
    }

    pub(crate) fn list_selection_key(
        &mut self,
        document: DocumentId,
        key: &str,
        modifiers: SelectionModifiers,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = self.world().focused(document) else {
            return Ok(false);
        };
        if self.world().text_input(focused).is_some() {
            return Ok(false);
        }
        let Some((list, item)) = enclosing_list_and_item(self, focused, false) else {
            return Ok(false);
        };
        let cols = self.read(list, |l| l.grid_columns.max(1)).unwrap_or(1);
        if modifiers.toggle && !modifiers.extend {
            match key {
                " " | "Space" => {
                    self.update_component(list, |_, view| {
                        view.emit(ListSelectionRequested {
                            item,
                            gesture: SelectionGesture::Toggle,
                            step: None,
                        });
                    })?;
                    return Ok(true);
                }
                _ if key.eq_ignore_ascii_case("a") => {
                    self.update_component(list, |_, view| {
                        view.emit(ListSelectionRequested {
                            item,
                            gesture: SelectionGesture::SelectAll,
                            step: None,
                        });
                    })?;
                    return Ok(true);
                }
                _ => {}
            }
        }
        let step = match key {
            "ArrowUp" => Some(if cols > 1 {
                SelectionStep::Backward(cols)
            } else {
                SelectionStep::Previous
            }),
            "ArrowDown" => Some(if cols > 1 {
                SelectionStep::Forward(cols)
            } else {
                SelectionStep::Next
            }),
            "ArrowLeft" if cols > 1 => Some(SelectionStep::Previous),
            "ArrowRight" if cols > 1 => Some(SelectionStep::Next),
            "Home" => Some(SelectionStep::First),
            "End" => Some(SelectionStep::Last),
            "PageUp" => Some(SelectionStep::Backward(LIST_PAGE_ITEMS * cols)),
            "PageDown" => Some(SelectionStep::Forward(LIST_PAGE_ITEMS * cols)),
            _ => None,
        };
        let Some(step) = step else {
            return Ok(false);
        };
        if self.world().is_virtual_list(list.stable_id()) {
            self.update_component(list, |_, view| {
                view.emit(ListSelectionRequested {
                    item,
                    gesture: SelectionGesture::Move(modifiers),
                    step: Some(step),
                });
            })?;
            return Ok(true);
        }
        let items = mounted_list_items(self, document, list.stable_id());
        let from = items.iter().position(|&id| id == item);
        let Some(target_idx) = step_in_mounted(&items, from, step) else {
            return Ok(false);
        };
        let target = items[target_idx];
        self.focus_node(document, target)?;
        self.update_component(list, |_, view| {
            view.emit(ListSelectionRequested {
                item: target,
                gesture: SelectionGesture::Move(modifiers),
                step: Some(step),
            });
        })?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use nana_ui_core::{SelectionState, keyed_order};
    use nana_ui_input::{
        InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey, PointerInput,
        PointerPhase,
    };

    use super::ListSelectionRequested;
    use crate::{
        AccessibilityRole, AppContext, DocumentId, Entity, HeadlessInput, LayoutViewport, List,
        ListItem,
    };

    const WIDTH: f32 = 320.0;
    const HEIGHT: f32 = 240.0;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn pointer(phase: PointerPhase, (x, y): (f32, f32), modifiers: InputModifiers) -> InputPayload {
        InputPayload::Pointer(PointerInput {
            modifiers,
            ..PointerInput::mouse(phase, x, y)
        })
    }

    fn key(name: &'static str, modifiers: InputModifiers) -> InputPayload {
        InputPayload::Key(KeyInput {
            physical: PhysicalKey(name.into()),
            logical: LogicalKey(name.into()),
            state: KeyState::Pressed,
            repeat: false,
            modifiers,
        })
    }

    const CTRL: InputModifiers = InputModifiers {
        control: true,
        ..InputModifiers::empty()
    };
    const SHIFT: InputModifiers = InputModifiers {
        shift: true,
        ..InputModifiers::empty()
    };
    const CTRL_SHIFT: InputModifiers = InputModifiers {
        control: true,
        shift: true,
        ..InputModifiers::empty()
    };

    #[derive(Clone)]
    struct RowData {
        id: &'static str,
        label: &'static str,
        disabled: bool,
    }

    struct PlainListFixture {
        cx: AppContext,
        list: Entity<List>,
        rows: Vec<(RowData, Entity<ListItem>)>,
        selection: SelectionState<&'static str>,
        heard: Arc<Mutex<Vec<ListSelectionRequested>>>,
    }

    impl PlainListFixture {
        fn new() -> Self {
            let mut cx = AppContext::new();
            let data = vec![
                RowData {
                    id: "a",
                    label: "Alpha",
                    disabled: false,
                },
                RowData {
                    id: "b",
                    label: "Beta",
                    disabled: false,
                },
                RowData {
                    id: "c",
                    label: "Gamma (disabled)",
                    disabled: true,
                },
                RowData {
                    id: "d",
                    label: "Delta",
                    disabled: false,
                },
                RowData {
                    id: "e",
                    label: "Epsilon",
                    disabled: false,
                },
            ];
            let list = cx
                .create_component(document(), List::new().label("Items").multiselectable(true))
                .unwrap();
            let mut rows = Vec::new();
            for row in data {
                let item = cx
                    .create_component(document(), ListItem::new(row.label).disabled(row.disabled))
                    .unwrap();
                cx.append_child(list, item).unwrap();
                rows.push((row, item));
            }
            let heard = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&heard);
            cx.on(list, move |_, event: &ListSelectionRequested, _| {
                sink.lock().unwrap().push(*event);
            })
            .unwrap();
            let mut fixture = Self {
                cx,
                list,
                rows,
                selection: SelectionState::multiple(),
                heard,
            };
            fixture.relayout();
            fixture
        }

        fn relayout(&mut self) {
            let work = self.cx.take_system_work();
            self.cx.resolve_styles(&work.style).unwrap();
            self.cx
                .layout_document(document(), LayoutViewport::new(WIDTH, HEIGHT))
                .unwrap();
            self.cx.rebuild_hit_test(document());
        }

        fn drain(&mut self) {
            let data = self
                .rows
                .iter()
                .map(|(row, _)| row.clone())
                .collect::<Vec<_>>();
            let order = keyed_order(&data, |r| r.id).selectable(|r| !r.disabled);
            for event in std::mem::take(&mut *self.heard.lock().unwrap()) {
                let Some(key) = self
                    .rows
                    .iter()
                    .find_map(|(r, entity)| (entity.stable_id() == event.item).then_some(r.id))
                else {
                    continue;
                };
                if let (Some(step), nana_ui_core::SelectionGesture::Move(mods)) =
                    (event.step, event.gesture)
                {
                    if self.selection.current().is_none()
                        && !matches!(
                            step,
                            nana_ui_core::SelectionStep::First | nana_ui_core::SelectionStep::Last
                        )
                    {
                        self.selection.set_current(key);
                    }
                    self.selection.navigate(step, mods, &order);
                } else {
                    self.selection.apply(event.gesture, key, &order);
                }
            }
            for (row, entity) in &self.rows {
                let selected = self.selection.is_selected(&row.id);
                if self.cx.read(*entity, |item| item.selected).unwrap() != selected {
                    self.cx
                        .update_component(*entity, |item, _| item.selected = selected)
                        .unwrap();
                }
            }
            self.relayout();
        }

        fn selected_ids(&self) -> Vec<&'static str> {
            let keys = self.rows.iter().map(|(r, _)| r.id).collect::<Vec<_>>();
            self.selection.selected_in(&keys)
        }
    }

    #[test]
    fn list_pointer_and_keyboard_selection_share_selection_state_contract() {
        let mut f = PlainListFixture::new();
        let mut input = HeadlessInput::bind(&mut f.cx, document());
        let center = |f: &PlainListFixture, idx: usize| {
            let b =
                f.cx.world()
                    .component_layout_box(f.rows[idx].1.stable_id())
                    .unwrap();
            (b.x + b.width / 2.0, b.y + b.height / 2.0)
        };

        // Click row 0 ("a")
        let p0 = center(&f, 0);
        input
            .route(
                &mut f.cx,
                pointer(PointerPhase::Down, p0, InputModifiers::empty()),
            )
            .unwrap();
        input
            .route(
                &mut f.cx,
                pointer(PointerPhase::Up, p0, InputModifiers::empty()),
            )
            .unwrap();
        f.drain();
        assert_eq!(f.selected_ids(), ["a"]);

        // Shift+click row 3 ("d") -> skips disabled "c"
        let p3 = center(&f, 3);
        input
            .route(&mut f.cx, pointer(PointerPhase::Down, p3, SHIFT))
            .unwrap();
        input
            .route(&mut f.cx, pointer(PointerPhase::Up, p3, SHIFT))
            .unwrap();
        f.drain();
        assert_eq!(f.selected_ids(), ["a", "b", "d"]);

        // Ctrl+click row 1 ("b") -> toggles off
        let p1 = center(&f, 1);
        input
            .route(&mut f.cx, pointer(PointerPhase::Down, p1, CTRL))
            .unwrap();
        input
            .route(&mut f.cx, pointer(PointerPhase::Up, p1, CTRL))
            .unwrap();
        f.drain();
        assert_eq!(f.selected_ids(), ["a", "d"]);

        // Keyboard: Ctrl+Shift+End extends additively from anchor "b" to "e"
        input.route(&mut f.cx, key("End", CTRL_SHIFT)).unwrap();
        f.drain();
        assert_eq!(f.selected_ids(), ["a", "b", "d", "e"]);

        // Ctrl+ArrowUp moves focus without selecting, Ctrl+Space toggles
        input.route(&mut f.cx, key("ArrowUp", CTRL)).unwrap();
        f.drain();
        assert_eq!(f.selected_ids(), ["a", "b", "d", "e"]);
        input.route(&mut f.cx, key(" ", CTRL)).unwrap();
        f.drain();
        assert_eq!(f.selected_ids(), ["a", "b", "e"]);

        // Ctrl+A selects all selectable items
        input.route(&mut f.cx, key("a", CTRL)).unwrap();
        f.drain();
        assert_eq!(f.selected_ids(), ["a", "b", "d", "e"]);

        // Accessibility reports multiselectable List and selected ListItems
        let spoken = f.cx.world().project_accessibility(document());
        let list_a11y = spoken
            .iter()
            .find(|node| node.id == f.list.stable_id())
            .unwrap();
        assert_eq!(list_a11y.role, AccessibilityRole::List);
        assert!(list_a11y.multiselectable);
        let item_a11y = spoken
            .iter()
            .find(|node| node.id == f.rows[0].1.stable_id())
            .unwrap();
        assert_eq!(item_a11y.selected, Some(true));
    }
}
