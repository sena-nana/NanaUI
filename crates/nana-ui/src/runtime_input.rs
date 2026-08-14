//! Stable platform-input routing for Nana-native Runtime components.

use nana_ui_core::TableNavigation;
use nana_ui_platform::{InputDisposition, InputEvent, PointerPhase};
use nana_ui_runtime::{AppContext, DocumentId, FrameworkError, ScrollOffset};

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
        let handled = match event {
            InputEvent::Pointer {
                phase,
                pointer_id,
                x,
                y,
                button,
                is_primary,
                ..
            } => {
                let target = context.pointer_target(document, *x, *y);
                context.set_pointer_hover(document, *pointer_id, target)?;
                match phase {
                    PointerPhase::Move => target.is_some(),
                    PointerPhase::Down if *is_primary && *button == 0 => {
                        if let Some(target) = target {
                            context.focus_node(document, target)?;
                            context.press_pointer(document, *pointer_id, target)?;
                            true
                        } else {
                            false
                        }
                    }
                    PointerPhase::Up if *is_primary && *button == 0 => {
                        let pressed = context.release_pointer(document, *pointer_id);
                        pressed == target
                            && target
                                .map(|target| context.activate_node(target))
                                .transpose()?
                                .unwrap_or(false)
                    }
                    PointerPhase::Cancel => {
                        let pressed = context.release_pointer(document, *pointer_id).is_some();
                        context.set_pointer_hover(document, *pointer_id, None)?;
                        pressed
                    }
                    _ => false,
                }
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
                context
                    .scroll_at(
                        document,
                        *x,
                        *y,
                        ScrollOffset {
                            x: -dx * scale,
                            y: -dy * scale,
                        },
                    )?
                    .is_some()
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
            prevent_default: handled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_platform::{InputModifiers, PointerType};
    use nana_ui_runtime::{
        Activate, Button, LayoutBox, MutationQueue, ScrollAxes, ScrollMetrics, ScrollView, Table,
        TableCell, TableRow, TextInput,
    };

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
            context
                .set_ime_preedit(document, "你".into(), None)
                .unwrap()
        );
        assert!(context.commit_ime(document, "你").unwrap());
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU你"));
        assert!(
            adapter
                .dispatch(&mut context, document, &key("Backspace"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU"));
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
}
