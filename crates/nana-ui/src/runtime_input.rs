//! Stable platform-input routing for Nana-native Runtime components.

use nana_ui_core::TableNavigation;
use nana_ui_platform::{ImeEvent, InputDisposition, InputEvent, PointerPhase};
use nana_ui_runtime::{
    AppContext, DocumentId, FrameworkError, GraphCanvasAdjustment, GraphPointerButton,
    GraphScrollDelta, RangeAdjustment, RovingFocusIntent, ScrollOffset, StableNodeId,
    XYPadAdjustment,
};
use nana_ui_runtime::{OverlayKey, OverlayPointerPhase};
use std::time::Duration;

const DEFAULT_LINE_SCROLL_EXTENT: f32 = 60.0;

/// Converts renderer-neutral platform input into typed Runtime component
/// actions. It owns no input or component state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeInputAdapter {
    pub line_scroll_extent: f32,
    pub table_page_rows: usize,
}

impl Default for RuntimeInputAdapter {
    fn default() -> Self {
        Self {
            line_scroll_extent: DEFAULT_LINE_SCROLL_EXTENT,
            table_page_rows: 10,
        }
    }
}

impl RuntimeInputAdapter {
    pub fn dispatch(
        self,
        context: &mut AppContext,
        document: DocumentId,
        event: &InputEvent,
    ) -> Result<InputDisposition, FrameworkError> {
        self.dispatch_at(context, document, event, Duration::ZERO)
    }

    /// Dispatch input at the host's monotonic Runtime timestamp. Timed
    /// component behavior such as tooltip delay uses this clock; no component
    /// owns a timer or requests frames while idle.
    pub fn dispatch_at(
        self,
        context: &mut AppContext,
        document: DocumentId,
        event: &InputEvent,
        now: Duration,
    ) -> Result<InputDisposition, FrameworkError> {
        let keyboard_barrier = matches!(event, InputEvent::Keyboard { .. })
            && context.has_blocking_runtime_overlay(document);
        if let InputEvent::Keyboard {
            pressed: true,
            key,
            repeat,
            modifiers,
            ..
        } = event
            && !modifiers.alt
            && !modifiers.control
            && !modifiers.meta
        {
            let overlay_key = match key.as_str() {
                "Escape" if !repeat => Some(OverlayKey::Escape),
                "Tab" => Some(OverlayKey::Tab {
                    reverse: modifiers.shift,
                }),
                _ => None,
            };
            if let Some(key) = overlay_key
                && context.route_overlay_key(document, key)?
            {
                return Ok(InputDisposition {
                    prevent_default: true,
                });
            }
        }
        if let InputEvent::Keyboard {
            pressed: true,
            key,
            repeat,
            modifiers,
            ..
        } = event
        {
            if key == "Tab"
                && !modifiers.alt
                && !modifiers.control
                && !modifiers.meta
                && context.navigate_sequential_focus(document, modifiers.shift)?
            {
                return Ok(InputDisposition {
                    prevent_default: true,
                });
            }
            if !modifiers.alt && !modifiers.control && !modifiers.meta && !modifiers.shift {
                let segmented_navigation = match key.as_str() {
                    "ArrowLeft" => Some(RovingFocusIntent::Previous),
                    "ArrowRight" => Some(RovingFocusIntent::Next),
                    "Home" => Some(RovingFocusIntent::First),
                    "End" => Some(RovingFocusIntent::Last),
                    _ => None,
                };
                if let Some(intent) = segmented_navigation
                    && context.navigate_focused_segmented(document, intent)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                if matches!(key.as_str(), " " | "Space" | "Enter")
                    && let Some(target) = context.world().focused(document)
                    && context.is_segmented_option_node(target)
                {
                    if !repeat {
                        context.activate_node(target)?;
                    }
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
            }
        }
        let handled = match event {
            InputEvent::Pointer {
                phase,
                pointer_id,
                x,
                y,
                button,
                is_primary,
                modifiers,
                ..
            } => {
                let overlay_phase = match phase {
                    PointerPhase::Move => OverlayPointerPhase::Move,
                    PointerPhase::Down if *is_primary && *button == 0 => {
                        OverlayPointerPhase::PrimaryDown
                    }
                    PointerPhase::Up if *is_primary && *button == 0 => {
                        OverlayPointerPhase::PrimaryUp
                    }
                    PointerPhase::Cancel => OverlayPointerPhase::Cancel,
                    PointerPhase::Down | PointerPhase::Up => OverlayPointerPhase::Move,
                };
                let overlay =
                    context.route_overlay_pointer(document, *pointer_id, overlay_phase, *x, *y)?;
                let target = overlay.target;
                context.set_pointer_location(document, *pointer_id, Some((*x, *y)));
                context.set_pointer_hover_at(document, *pointer_id, target, now)?;
                let graph_button = match *button {
                    1 => GraphPointerButton::Middle,
                    _ => GraphPointerButton::Primary,
                };
                let component_handled = match phase {
                    PointerPhase::Move => {
                        context.update_range_drag(document, *pointer_id, *x)?
                            || context.update_xy_pad_drag(
                                document,
                                *pointer_id,
                                *x,
                                *y,
                                modifiers.shift,
                            )?
                            || context.update_graph_canvas_pointer(document, *pointer_id, *x, *y)?
                            || context.update_split_resize(document, *pointer_id, *x, *y)?
                            || context.update_dock_split_resize(document, *pointer_id, *x, *y)?
                            || context.update_dock_item_drag(document, *pointer_id, *x, *y)?
                            || target
                                .map(|target| context.hover_graph_canvas(target, *x, *y))
                                .transpose()?
                                .unwrap_or(false)
                            || target
                                .map(|target| context.hover_calendar_heatmap(target, *x, *y))
                                .transpose()?
                                .unwrap_or(false)
                            || context.clear_calendar_heatmap_hover(document)?
                            || context.hover_split_handle(
                                context.split_handle_near(document, *x, *y).or(target),
                            )?
                            || target.is_some()
                    }
                    PointerPhase::Down if (*is_primary && *button == 0) || *button == 1 => {
                        context.dismiss_detached_menus(target)?;
                        let split_handle = context.split_handle_near(document, *x, *y);
                        let dock_handle = context.dock_handle_near(document, *x, *y);
                        let dock_source = target
                            .filter(|id| context.is_dock_item_source(*id))
                            .or_else(|| context.dock_tab_strip_near(document, *x, *y));
                        if let Some(target) = dock_handle.or(split_handle).or(target) {
                            if let Some(focus) = nearest_focusable(context, target) {
                                context.focus_node(document, focus)?;
                            }
                            if context.is_graph_canvas(target) {
                                context.begin_graph_canvas_pointer(
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                    graph_button,
                                )?;
                            } else if context.is_dock_handle(target) && *button == 0 {
                                context.begin_dock_split_resize(
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                )?;
                            } else if context.is_split_handle(target) && *button == 0 {
                                context.begin_split_resize(
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                )?;
                            } else if *button == 0 {
                                if let Some(source) = dock_source {
                                    context.begin_dock_item_drag(
                                        document,
                                        *pointer_id,
                                        source,
                                        *x,
                                        *y,
                                    )?;
                                } else {
                                    context.press_pointer(document, *pointer_id, target)?;
                                    if context.is_range_field(target) {
                                        context.begin_range_drag(
                                            document,
                                            *pointer_id,
                                            target,
                                            *x,
                                        )?;
                                    } else if context.is_xy_pad(target) {
                                        context.begin_xy_pad_drag(
                                            document,
                                            *pointer_id,
                                            target,
                                            *x,
                                            *y,
                                        )?;
                                    }
                                }
                            }
                            true
                        } else {
                            false
                        }
                    }
                    PointerPhase::Up if (*is_primary && *button == 0) || *button == 1 => {
                        if context.end_range_drag(document, *pointer_id, false)?
                            || context.end_xy_pad_drag(document, *pointer_id, false)?
                            || context.end_graph_canvas_pointer(
                                document,
                                *pointer_id,
                                *x,
                                *y,
                                false,
                            )?
                            || context.end_split_resize(document, *pointer_id, false)?
                            || context.end_dock_split_resize(document, *pointer_id, false)?
                            || context.end_dock_item_drag(document, *pointer_id, *x, *y, false)?
                        {
                            context.release_pointer(document, *pointer_id);
                            return Ok(InputDisposition {
                                prevent_default: true,
                            });
                        }
                        let pressed = context.release_pointer(document, *pointer_id);
                        if let Some(pressed) = pressed {
                            if Some(pressed) == target {
                                context.activate_node_at(pressed, *x, *y)?;
                            }
                            true
                        } else {
                            false
                        }
                    }
                    PointerPhase::Cancel => {
                        let range = context.end_range_drag(document, *pointer_id, true)?;
                        let xy_pad = context.end_xy_pad_drag(document, *pointer_id, true)?;
                        let graph = context.end_graph_canvas_pointer(
                            document,
                            *pointer_id,
                            *x,
                            *y,
                            true,
                        )?;
                        let split = context.end_split_resize(document, *pointer_id, true)?;
                        let dock_split =
                            context.end_dock_split_resize(document, *pointer_id, true)?;
                        let dock_item =
                            context.end_dock_item_drag(document, *pointer_id, *x, *y, true)?;
                        let pressed = context.release_pointer(document, *pointer_id).is_some();
                        context.set_pointer_hover_at(document, *pointer_id, None, now)?;
                        let calendar = context.clear_calendar_heatmap_hover(document)?;
                        range
                            || xy_pad
                            || graph
                            || split
                            || dock_split
                            || dock_item
                            || calendar
                            || pressed
                    }
                    _ => false,
                };
                overlay.prevent_default || component_handled
            }
            InputEvent::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                line_delta,
                modifiers,
            } => {
                let (dx, dy) = if modifiers.shift && !cfg!(target_os = "macos") {
                    (*delta_y, *delta_x)
                } else {
                    (*delta_x, *delta_y)
                };
                let scale = if *line_delta {
                    self.line_scroll_extent
                } else {
                    1.0
                };
                let delta = ScrollOffset {
                    x: -dx * scale,
                    y: -dy * scale,
                };
                let overlay = context.route_overlay_pointer(
                    document,
                    0,
                    OverlayPointerPhase::Wheel,
                    *x,
                    *y,
                )?;
                let graph_delta = if *line_delta {
                    GraphScrollDelta::Lines { y: -dy }
                } else {
                    GraphScrollDelta::Pixels { y: -dy }
                };
                let graph_target = context.pointer_target(document, *x, *y);
                let scrolled = if overlay.prevent_default {
                    overlay
                        .target
                        .map(|target| context.scroll_overlay_from(document, target, delta))
                        .transpose()?
                        .flatten()
                        .is_some()
                } else if graph_target.is_some_and(|target| context.is_graph_canvas(target)) {
                    context.scroll_graph_canvas(
                        document,
                        graph_target.expect("graph target"),
                        *x,
                        *y,
                        graph_delta,
                    )?
                } else {
                    context.scroll_at(document, *x, *y, delta)?.is_some()
                };
                overlay.prevent_default || scrolled
            }
            InputEvent::Keyboard {
                pressed,
                key,
                text,
                repeat: _,
                modifiers,
                ..
            } if *pressed && !modifiers.alt && !modifiers.shift => {
                let primary = modifiers.control || modifiers.meta;
                let range_adjustment = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" | "ArrowDown" => Some(RangeAdjustment::Decrement),
                        "ArrowRight" | "ArrowUp" => Some(RangeAdjustment::Increment),
                        "PageDown" => Some(RangeAdjustment::PageDecrement),
                        "PageUp" => Some(RangeAdjustment::PageIncrement),
                        "Home" => Some(RangeAdjustment::Minimum),
                        "End" => Some(RangeAdjustment::Maximum),
                        _ => None,
                    })
                    .flatten();
                if let Some(adjustment) = range_adjustment
                    && context.adjust_focused_range(document, adjustment)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                let xy_adjustment = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" => Some(XYPadAdjustment::Left),
                        "ArrowRight" => Some(XYPadAdjustment::Right),
                        "ArrowUp" => Some(XYPadAdjustment::Up),
                        "ArrowDown" => Some(XYPadAdjustment::Down),
                        _ => None,
                    })
                    .flatten();
                if let Some(adjustment) = xy_adjustment
                    && context.adjust_focused_xy_pad(document, adjustment)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                let graph_adjustment = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" => Some(GraphCanvasAdjustment::PanLeft),
                        "ArrowRight" => Some(GraphCanvasAdjustment::PanRight),
                        "ArrowUp" => Some(GraphCanvasAdjustment::PanUp),
                        "ArrowDown" => Some(GraphCanvasAdjustment::PanDown),
                        "Home" | "0" => Some(GraphCanvasAdjustment::Fit),
                        "+" | "=" => Some(GraphCanvasAdjustment::ZoomIn),
                        "-" => Some(GraphCanvasAdjustment::ZoomOut),
                        "Escape" => Some(GraphCanvasAdjustment::ClearSelection),
                        _ => None,
                    })
                    .flatten();
                if let Some(adjustment) = graph_adjustment
                    && context.adjust_focused_graph_canvas(document, adjustment)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                let split_direction = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" | "ArrowUp" => Some(-1.0),
                        "ArrowRight" | "ArrowDown" => Some(1.0),
                        _ => None,
                    })
                    .flatten();
                if let Some(direction) = split_direction
                    && (context.adjust_focused_split(document, direction)?
                        || context.adjust_focused_dock_split(document, direction)?)
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                if !primary {
                    let palette_nav = match key.as_str() {
                        "ArrowUp" => Some(nana_ui_runtime::ActionPickerNavigation::Previous),
                        "ArrowDown" => Some(nana_ui_runtime::ActionPickerNavigation::Next),
                        "Home" => Some(nana_ui_runtime::ActionPickerNavigation::First),
                        "End" => Some(nana_ui_runtime::ActionPickerNavigation::Last),
                        "Enter" => Some(nana_ui_runtime::ActionPickerNavigation::Confirm),
                        "Escape" => Some(nana_ui_runtime::ActionPickerNavigation::Dismiss),
                        _ => None,
                    };
                    if let Some(navigation) = palette_nav
                        && context.navigate_focused_command_palette(document, navigation)?
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    let select_delta = match key.as_str() {
                        "ArrowUp" => Some(-1),
                        "ArrowDown" => Some(1),
                        _ => None,
                    };
                    if let Some(delta) = select_delta
                        && (context.adjust_focused_select(document, delta)?
                            || context.adjust_focused_dropdown(document, delta)?
                            || context.adjust_focused_search_dropdown(document, delta)?)
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    if matches!(key.as_str(), " " | "Space" | "Enter")
                        && (context.commit_focused_select(document)?
                            || context.commit_focused_dropdown(document)?)
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    if matches!(key.as_str(), "Enter")
                        && context.commit_focused_search_dropdown(document)?
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                    let tree_nav = match key.as_str() {
                        "ArrowUp" => Some(nana_ui_runtime::TreeNavigation::Previous),
                        "ArrowDown" => Some(nana_ui_runtime::TreeNavigation::Next),
                        "Home" => Some(nana_ui_runtime::TreeNavigation::First),
                        "End" => Some(nana_ui_runtime::TreeNavigation::Last),
                        "ArrowLeft" => Some(nana_ui_runtime::TreeNavigation::Parent),
                        "ArrowRight" => Some(nana_ui_runtime::TreeNavigation::Child),
                        "Enter" => Some(nana_ui_runtime::TreeNavigation::Activate),
                        " " | "Space" => Some(nana_ui_runtime::TreeNavigation::Toggle),
                        _ => None,
                    };
                    if let Some(navigation) = tree_nav
                        && context.navigate_focused_tree(document, navigation)?
                    {
                        return Ok(InputDisposition {
                            prevent_default: true,
                        });
                    }
                }
                if !primary
                    && matches!(key.as_str(), " " | "Space" | "Enter")
                    && let Some(target) = context.world().focused(document)
                    && context.activate_node(target)?
                {
                    return Ok(InputDisposition {
                        prevent_default: true,
                    });
                }
                let navigation = match (key.as_str(), primary) {
                    ("ArrowUp", false) => Some(TableNavigation::PreviousRow),
                    ("ArrowDown", false) => Some(TableNavigation::NextRow),
                    ("ArrowLeft", false) => Some(TableNavigation::PreviousColumn),
                    ("ArrowRight", false) => Some(TableNavigation::NextColumn),
                    ("Home", false) => Some(TableNavigation::RowStart),
                    ("End", false) => Some(TableNavigation::RowEnd),
                    ("Home", true) => Some(TableNavigation::FirstRow),
                    ("End", true) => Some(TableNavigation::LastRow),
                    ("PageUp", false) => Some(TableNavigation::PageUp),
                    ("PageDown", false) => Some(TableNavigation::PageDown),
                    _ => None,
                };
                if let Some(navigation) = navigation {
                    context.navigate_focused_table(document, navigation, self.table_page_rows)?
                } else if !primary && !modifiers.alt {
                    match key.as_str() {
                        "Backspace" => context.delete_focused_text_backward(document)?,
                        _ if text.as_ref().is_some_and(|text| !text.is_empty()) => context
                            .replace_focused_text(document, text.as_deref().unwrap_or_default())?,
                        _ => false,
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        Ok(InputDisposition {
            prevent_default: handled || keyboard_barrier,
        })
    }

    /// Route platform IME into the focused Runtime editor.
    ///
    /// Retained TextInput/TextArea/SearchDropdown/CommandPalette state is the
    /// only editing authority. A
    /// focused editable field, or a blocking overlay, consumes the event so a
    /// second host IME path cannot also mutate it.
    pub fn dispatch_ime(
        self,
        context: &mut AppContext,
        document: DocumentId,
        event: &ImeEvent,
    ) -> Result<InputDisposition, FrameworkError> {
        let overlay_blocks = context.has_blocking_runtime_overlay(document);
        let owns_ime = context
            .focused_text_input(document)
            .is_some_and(|(target, _)| {
                context
                    .world()
                    .accessibility(target)
                    .is_some_and(|state| state.editable)
            });
        let handled = match event {
            ImeEvent::Enabled => false,
            ImeEvent::Disabled => {
                let leftover = context
                    .world()
                    .focused_text_input(document)
                    .and_then(|(id, _)| context.world().ime(id).map(|ime| ime.text.clone()))
                    .filter(|text| !text.is_empty());
                match leftover {
                    Some(text) => context.commit_ime(document, &text)?,
                    None => context.clear_ime(document)?,
                }
            }
            ImeEvent::Preedit { text, selection } => {
                context.set_ime_preedit(document, text.clone(), *selection)?
            }
            ImeEvent::Commit(text) => context.commit_ime(document, text)?,
        };
        Ok(InputDisposition {
            prevent_default: handled || owns_ime || overlay_blocks,
        })
    }
}

fn nearest_focusable(context: &AppContext, mut target: StableNodeId) -> Option<StableNodeId> {
    loop {
        if context
            .world()
            .interaction(target)
            .is_some_and(|interaction| interaction.focusable)
        {
            return Some(target);
        }
        target = context.world().node(target).and_then(|node| node.parent)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_platform::{ImeEvent, InputModifiers, PointerType};
    use nana_ui_runtime::{
        Activate, Button, CalendarHeatmap, CalendarHeatmapDatum, Dialog, Dock, DockAxis, DockNode,
        LayoutBox, Menu, MenuItem, ModalSlots, MutationQueue, OverlayHost, OverlayHostState,
        RangeField, ScrollAxes, ScrollMetrics, ScrollView, SegmentedControl, SegmentedOption,
        SegmentedSelectionRequested, Table, TableCell, TableRow, Text, TextArea, TextInput,
    };
    use std::sync::{Arc, Mutex};

    fn wheel(x: f32, y: f32, delta_y: f32) -> InputEvent {
        InputEvent::Wheel {
            x,
            y,
            delta_x: 0.0,
            delta_y,
            line_delta: true,
            modifiers: InputModifiers::default(),
        }
    }

    fn focused_untyped_text_input(
        context: &mut AppContext,
        value: &str,
    ) -> (DocumentId, nana_ui_runtime::StableNodeId) {
        let document = DocumentId::new(1).unwrap();
        let id = nana_ui_runtime::StableNodeId::new(1).unwrap();
        let mut create = MutationQueue::new();
        create.create(
            id,
            document,
            nana_ui_runtime::NodeKind::Element {
                tag: "input".into(),
            },
        );
        create.set_interaction(
            id,
            nana_ui_runtime::InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        create.set_text_input(id, Some(nana_ui_runtime::TextInputState::new(value)));
        create.set_accessibility(
            id,
            nana_ui_runtime::AccessibilityState {
                role: nana_ui_runtime::AccessibilityRole::TextInput,
                editable: true,
                ..nana_ui_runtime::AccessibilityState::default()
            },
        );
        create.request_focus(document, Some(id));
        context.commit_mutations(create).unwrap();
        (document, id)
    }

    fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputEvent {
        InputEvent::Pointer {
            phase,
            pointer_id: 1,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase == PointerPhase::Down),
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            modifiers: InputModifiers::default(),
        }
    }

    #[test]
    fn pointer_release_activates_the_retained_hit_target() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Build"))
            .unwrap();
        context
            .on(button, |button, _event: &Activate, _cx| {
                button.label = "Running".into();
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 120.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 30.0, 30.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 30.0, 30.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(button.stable_id()), Some("Running"));
    }

    #[test]
    fn pointer_down_moves_focus_to_the_hit_text_input() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Other"))
            .unwrap();
        let input = context
            .create_component(document, TextInput::new("NanaUI"))
            .unwrap();
        assert!(context.focus_node(document, button.stable_id()).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 32.0,
            },
        );
        layout.write_layout(
            input.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 40.0,
                width: 160.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 24.0, 52.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(input.stable_id()));
    }

    #[test]
    fn segmented_pointer_lease_consumes_release_and_only_requests_on_inside_up() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();
        let requests = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&requests);
        context
            .on(
                control,
                move |_control, _event: &SegmentedSelectionRequested, _cx| {
                    *observed.lock().unwrap() += 1;
                },
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            control.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 180.0,
                height: 32.0,
            },
        );
        layout.write_layout(
            first.stable_id(),
            LayoutBox {
                x: 4.0,
                y: 3.0,
                width: 70.0,
                height: 26.0,
            },
        );
        layout.write_layout(
            second.stable_id(),
            LayoutBox {
                x: 76.0,
                y: 3.0,
                width: 70.0,
                height: 26.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        let adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 90.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
        assert_eq!(
            context
                .read(control, SegmentedControl::focus_target)
                .unwrap(),
            Some(second.stable_id())
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Cancel, 90.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 0);

        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 140.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 0);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 1);
        assert!(context.read(first, SegmentedOption::selected).unwrap());
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Cancel, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 1);
    }

    #[test]
    fn document_tab_order_uses_one_roving_entry_and_wraps_both_directions() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let before = context
            .create_component(document, Button::new("Before"))
            .unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        let after = context
            .create_component(document, Button::new("After"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();
        context.focus_node(document, before.stable_id()).unwrap();
        let tab = |shift| InputEvent::Keyboard {
            pressed: true,
            key: "Tab".into(),
            text: None,
            code: "Tab".into(),
            repeat: false,
            modifiers: InputModifiers {
                shift,
                ..InputModifiers::default()
            },
        };
        let adapter = RuntimeInputAdapter::default();
        for expected in [first.stable_id(), after.stable_id(), before.stable_id()] {
            assert!(
                adapter
                    .dispatch(&mut context, document, &tab(false))
                    .unwrap()
                    .prevent_default
            );
            assert_eq!(context.world().focused(document), Some(expected));
        }
        assert!(
            adapter
                .dispatch(&mut context, document, &tab(true))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(after.stable_id()));
        assert!(context.focus_node(document, second.stable_id()).unwrap());
    }

    #[test]
    fn focused_runtime_text_uses_keyboard_and_ime_state() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("Nana"))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());
        let key = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: (key.chars().count() == 1).then(|| key.into()),
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &key("U"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU"));
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "你".into(),
                        selection: None,
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .ime(input.stable_id())
                .map(|ime| ime.text.as_str()),
            Some("你")
        );
        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("你".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU你"));
        assert_eq!(context.world().ime(input.stable_id()), None);
        assert!(
            adapter
                .dispatch(&mut context, document, &key("Backspace"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU"));
    }

    #[test]
    fn focused_runtime_textarea_ime_updates_multiline_state() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("第一行\n"))
            .unwrap();
        assert!(context.focus_node(document, area.stable_id()).unwrap());

        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "第二".into(),
                        selection: Some((0, "第".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        let composition = context
            .world()
            .ime(area.stable_id())
            .expect("focused textarea keeps preedit on retained state");
        assert_eq!(composition.text, "第二");
        assert_eq!(composition.selection, Some((0, "第".len())));
        assert_eq!(context.world().text(area.stable_id()), Some("第一行\n"));

        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("第二行".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text(area.stable_id()),
            Some("第一行\n第二行")
        );
        assert_eq!(context.world().ime(area.stable_id()), None);

        context
            .update_component(area, |area, _cx| area.disabled = true)
            .unwrap();
        assert!(
            !adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "三".into(),
                        selection: None,
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text(area.stable_id()),
            Some("第一行\n第二行")
        );
        assert_eq!(context.world().ime(area.stable_id()), None);
    }

    #[test]
    fn dispatch_ime_commits_a_focused_text_input_without_a_typed_view() {
        let mut context = AppContext::new();
        let (document, id) = focused_untyped_text_input(&mut context, "Nana");
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "世".into(),
                        selection: Some((0, "世".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().ime(id).map(|ime| ime.text.as_str()),
            Some("世")
        );
        assert_eq!(
            context
                .world()
                .text_input(id)
                .map(|state| state.value.as_str()),
            Some("Nana")
        );

        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("世界".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .text_input(id)
                .map(|state| state.value.as_str()),
            Some("Nana世界")
        );
        assert!(context.world().ime(id).is_none());
    }

    #[test]
    fn dispatch_ime_disabled_commits_leftover_preedit_without_a_typed_view() {
        let mut context = AppContext::new();
        let (document, id) = focused_untyped_text_input(&mut context, "Nana");
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "世".into(),
                        selection: Some((0, "世".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Disabled)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .text_input(id)
                .map(|state| state.value.as_str()),
            Some("Nana世")
        );
        assert!(context.world().ime(id).is_none());
    }

    #[test]
    fn wheel_routes_to_nearest_scrollview_and_bubbles_at_edge() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let outer = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        let inner = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        let cell = context
            .create_component(document, TableCell::new("row"))
            .unwrap();
        context.append_child(outer, inner).unwrap();
        context.append_child(inner, cell).unwrap();
        context
            .set_scroll_metrics(
                outer,
                ScrollMetrics {
                    viewport_width: 200.0,
                    viewport_height: 100.0,
                    content_width: 200.0,
                    content_height: 300.0,
                },
            )
            .unwrap();
        context
            .set_scroll_metrics(
                inner,
                ScrollMetrics {
                    viewport_width: 180.0,
                    viewport_height: 80.0,
                    content_width: 180.0,
                    content_height: 140.0,
                },
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            outer.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
        );
        layout.write_layout(
            inner.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 180.0,
                height: 80.0,
            },
        );
        layout.write_layout(
            cell.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 160.0,
                height: 30.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(10.0, 10.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().scroll_offset(inner.stable_id()).unwrap().y,
            60.0
        );
        assert_eq!(
            context.world().scroll_offset(outer.stable_id()).unwrap().y,
            0.0
        );
        context.take_system_work();
        context.rebuild_hit_test(document);

        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(10.0, 10.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().scroll_offset(inner.stable_id()).unwrap().y,
            60.0
        );
        assert_eq!(
            context.world().scroll_offset(outer.stable_id()).unwrap().y,
            60.0
        );
    }

    #[test]
    fn keyboard_routes_navigation_from_focused_table_cell() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let table = context.create_component(document, Table::new()).unwrap();
        let first_row = context.create_component(document, TableRow::new()).unwrap();
        let second_row = context.create_component(document, TableRow::new()).unwrap();
        let first = context
            .create_component(document, TableCell::new("one"))
            .unwrap();
        let second = context
            .create_component(document, TableCell::new("two"))
            .unwrap();
        context.append_child(table, first_row).unwrap();
        context.append_child(table, second_row).unwrap();
        context.append_child(first_row, first).unwrap();
        context.append_child(second_row, second).unwrap();
        let mut focus = MutationQueue::new();
        focus.request_focus(document, Some(first.stable_id()));
        context.commit_mutations(focus).unwrap();

        let event = InputEvent::Keyboard {
            pressed: true,
            key: "ArrowDown".into(),
            text: None,
            code: "ArrowDown".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(&mut context, document, &event)
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
    }

    #[test]
    fn segmented_keyboard_routing_precedes_generic_navigation_and_activation() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let disabled = context
            .create_detached_component(document, SegmentedOption::new("Split").disabled(true))
            .unwrap();
        let last = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, disabled, last], Some(first))
            .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        context
            .on(
                control,
                move |_control, event: &SegmentedSelectionRequested, _cx| {
                    observed.lock().unwrap().push(event.option);
                },
            )
            .unwrap();
        context.focus_node(document, first.stable_id()).unwrap();
        let key = |key: &str, repeat: bool, modifiers: InputModifiers| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat,
            modifiers,
        };
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("ArrowRight", false, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(last.stable_id()));
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(first.stable_id())
        );
        assert_eq!(&*requests.lock().unwrap(), &[last.stable_id()]);
        for modifiers in [
            InputModifiers {
                alt: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        ] {
            assert!(
                !adapter
                    .dispatch(&mut context, document, &key("Home", true, modifiers))
                    .unwrap()
                    .prevent_default
            );
        }
        assert_eq!(context.world().focused(document), Some(last.stable_id()));
        assert_eq!(requests.lock().unwrap().as_slice(), [last.stable_id()]);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("Home", true, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(first.stable_id()));
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [last.stable_id(), first.stable_id()]
        );
        let count = requests.lock().unwrap().len();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("Space", true, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(requests.lock().unwrap().len(), count);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("Enter", false, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(requests.lock().unwrap().last(), Some(&first.stable_id()));
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(first.stable_id())
        );
        assert!(
            context
                .set_segmented_selection(control, Some(last))
                .unwrap()
        );
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(last.stable_id())
        );
    }

    #[test]
    fn range_field_quantizes_keyboard_and_cancels_captured_drag() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let range = context
            .create_component(document, RangeField::new(0.5, 0.0, 1.0, 0.1).unwrap())
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            range.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 300.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        assert!(context.focus_node(document, range.stable_id()).unwrap());

        let key = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &key("ArrowRight"))
                .unwrap()
                .prevent_default
        );
        assert!((context.read(range, |range| range.value).unwrap() - 0.6).abs() < 1e-12);
        assert!(
            adapter
                .dispatch(&mut context, document, &key("Home"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.0);

        let track = match context.world().component_geometry(range.stable_id()) {
            Some(nana_ui_runtime::ComponentGeometry::Range { track, .. }) => track,
            _ => panic!("range geometry expected"),
        };
        let drag_x = track.x + track.width * 0.8;
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, drag_x, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.8);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Cancel, drag_x, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.0);
        assert_eq!(context.world().pointer_capture(document, 1), None);
    }

    #[test]
    fn overlay_pointer_sequence_never_activates_the_underlay() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let underlay = context
            .create_component(document, Button::new("Underlay"))
            .unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Dialog"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        let activations = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&activations);
        context
            .on(underlay, move |_button, _event: &Activate, _cx| {
                *observed.lock().unwrap() += 1;
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            underlay.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 300.0,
            },
        );
        layout.write_layout(
            dialog.stable_id(),
            LayoutBox {
                x: 100.0,
                y: 100.0,
                width: 100.0,
                height: 100.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.activate_overlay(host, dialog).unwrap();
        context.rebuild_hit_test(document);

        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 20.0),
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 20.0, 20.0),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*activations.lock().unwrap(), 0);
    }

    #[test]
    fn menu_item_can_close_during_activation_without_releasing_input_or_wheel_barrier() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let background = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let menu = context.create_component(document, Menu::new()).unwrap();
        let item = context
            .create_component(document, MenuItem::new("Build"))
            .unwrap();
        context.append_child(background, host).unwrap();
        context.append_child(host, menu).unwrap();
        context.append_child(menu, item).unwrap();
        let host_id = host.stable_id();
        context
            .on(item, move |_item, _event: &Activate, cx| {
                cx.mutations()
                    .set_overlay_host(host_id, OverlayHostState::default());
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        for (id, x, y, width, height) in [
            (background.stable_id(), 0.0, 0.0, 300.0, 300.0),
            (menu.stable_id(), 100.0, 100.0, 100.0, 100.0),
            (item.stable_id(), 110.0, 110.0, 80.0, 32.0),
        ] {
            layout.write_layout(
                id,
                LayoutBox {
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
        context.commit_mutations(layout).unwrap();
        context
            .set_scroll_metrics(
                background,
                ScrollMetrics {
                    viewport_width: 300.0,
                    viewport_height: 300.0,
                    content_width: 300.0,
                    content_height: 900.0,
                },
            )
            .unwrap();
        context.activate_overlay(host, menu).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        context.rebuild_hit_test(document);
        let adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(20.0, 20.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .scroll_offset(background.stable_id())
                .unwrap()
                .y,
            0.0
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 120.0, 120.0),
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 120.0, 120.0),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            None
        );
    }

    #[test]
    fn overlay_keyboard_ignores_primary_tab_and_repeated_escape() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Settings"))
            .unwrap();
        let button = context
            .create_detached_component(document, Button::new("Save"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        context
            .set_modal_slots(
                dialog,
                ModalSlots {
                    actions: vec![button.stable_id()],
                    ..ModalSlots::default()
                },
            )
            .unwrap();
        context.activate_overlay(host, dialog).unwrap();
        let adapter = RuntimeInputAdapter::default();
        let key = |key: &str, repeat: bool, modifiers: InputModifiers| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat,
            modifiers,
        };

        let primary_tab = key(
            "Tab",
            false,
            InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary_tab)
                .unwrap()
                .prevent_default
        );
        let repeat_escape = key("Escape", true, InputModifiers::default());
        assert!(
            adapter
                .dispatch(&mut context, document, &repeat_escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            Some(dialog.stable_id())
        );
        let key_release = InputEvent::Keyboard {
            pressed: false,
            key: "a".into(),
            text: None,
            code: "KeyA".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert!(
            adapter
                .dispatch(&mut context, document, &key_release)
                .unwrap()
                .prevent_default
        );
        let escape = key("Escape", false, InputModifiers::default());
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            None
        );
        assert!(
            !adapter
                .dispatch(&mut context, document, &primary_tab)
                .unwrap()
                .prevent_default
        );
    }

    #[test]
    fn pointer_on_dock_handle_changes_split_ratio() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let first = context
            .create_component(document, Text::new("first"))
            .unwrap()
            .stable_id();
        let second = context
            .create_component(document, Text::new("second"))
            .unwrap()
            .stable_id();
        let dock = context
            .create_component(
                document,
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let handle = context.world().node(dock.stable_id()).unwrap().children[1];
        let mut layout = MutationQueue::new();
        layout.write_layout(
            dock.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            handle,
            LayoutBox {
                x: 156.8,
                y: 0.0,
                width: 8.0,
                height: 200.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);

        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 160.0, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Move, 200.0, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 200.0, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        let ratio = context
            .read(dock, |dock| match &dock.root {
                DockNode::Split { ratio, .. } => *ratio,
                _ => panic!("split"),
            })
            .unwrap();
        assert!((ratio - (0.4_f32 + 40.0 / 392.0).clamp(0.05, 0.95)).abs() < 0.001);
    }

    #[test]
    fn keyboard_arrow_right_on_focused_dock_handle_changes_ratio() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let first = context
            .create_component(document, Text::new("first"))
            .unwrap()
            .stable_id();
        let second = context
            .create_component(document, Text::new("second"))
            .unwrap()
            .stable_id();
        let dock = context
            .create_component(
                document,
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let handle = context.world().node(dock.stable_id()).unwrap().children[1];
        assert!(context.focus_node(document, handle).unwrap());

        let event = InputEvent::Keyboard {
            pressed: true,
            key: "ArrowRight".into(),
            text: None,
            code: "ArrowRight".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(&mut context, document, &event)
                .unwrap()
                .prevent_default
        );
        let ratio = context
            .read(dock, |dock| match &dock.root {
                DockNode::Split { ratio, .. } => *ratio,
                _ => panic!("split"),
            })
            .unwrap();
        assert!((ratio - 0.45).abs() < 0.001);
    }

    #[test]
    fn pointer_on_calendar_heatmap_sets_active_cell() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let heatmap = context
            .create_component(
                document,
                CalendarHeatmap::new([
                    CalendarHeatmapDatum::<()>::new("2026-06-01", 2.0),
                    CalendarHeatmapDatum::<()>::new("2026-06-03", 8.0),
                ]),
            )
            .unwrap();
        let model = context.read(heatmap, CalendarHeatmap::model).unwrap();
        let cell = model
            .cells
            .iter()
            .find(|cell| cell.date == "2026-06-03")
            .expect("June 3");
        context
            .commit_mutations({
                let mut mutations = MutationQueue::new();
                mutations.write_layout(
                    heatmap.stable_id(),
                    LayoutBox {
                        x: 0.0,
                        y: 0.0,
                        width: model.width,
                        height: model.height,
                    },
                );
                mutations
            })
            .unwrap();
        context.rebuild_hit_test(document);

        assert!(
            RuntimeInputAdapter::default()
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Move, cell.x + 1.0, cell.y + 1.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.read(heatmap, |calendar| calendar.active).unwrap(),
            Some(
                model
                    .cells
                    .iter()
                    .position(|item| item.date == "2026-06-03")
                    .expect("index")
            )
        );
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Move, 400.0, 400.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            context
                .read(heatmap, |calendar| calendar.active)
                .unwrap()
                .is_none()
        );
    }
}
