use std::rc::Rc;

use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector, keyboard};

use crate::KeyStroke;

/// A typed shortcut-capture outcome. The application owns persistence and
/// decides when to leave capture mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCaptureEvent {
    Captured(KeyStroke),
    Cleared,
    Cancelled,
}

/// Captures the next shortcut before child inputs or the application keymap.
///
/// Render this layer only while the user is explicitly recording a shortcut.
/// Modifier-only presses stay pending; Tab keeps normal focus traversal.
pub struct KeyCaptureLayer<'a, Message> {
    content: Element<'a, Message>,
    on_event: Rc<dyn Fn(KeyCaptureEvent) -> Message + 'a>,
}

impl<'a, Message> KeyCaptureLayer<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        on_event: impl Fn(KeyCaptureEvent) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            on_event: Rc::new(on_event),
        }
    }

    pub fn view(self) -> Element<'a, Message> {
        self.into()
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for KeyCaptureLayer<'_, Message>
where
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        self.content.as_widget_mut().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        match resolve_capture_event(event) {
            CaptureResolution::Publish(event) => {
                shell.publish((self.on_event)(event));
                shell.capture_event();
                return;
            }
            CaptureResolution::Pending => {
                shell.capture_event();
                return;
            }
            CaptureResolution::Pass => {}
        }
        self.content
            .as_widget_mut()
            .update(tree, event, layout, cursor, renderer, shell, viewport);
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

impl<'a, Message> From<KeyCaptureLayer<'a, Message>> for Element<'a, Message>
where
    Message: Clone + 'a,
{
    fn from(layer: KeyCaptureLayer<'a, Message>) -> Self {
        Element::new(layer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CaptureResolution {
    Publish(KeyCaptureEvent),
    Pending,
    Pass,
}

fn resolve_capture_event(event: &Event) -> CaptureResolution {
    let Event::Keyboard(keyboard::Event::KeyPressed {
        key,
        modifiers,
        repeat,
        ..
    }) = event
    else {
        return CaptureResolution::Pass;
    };
    if *repeat {
        return CaptureResolution::Pending;
    }
    use keyboard::key::Named;
    match key.as_ref() {
        keyboard::Key::Named(Named::Tab) => CaptureResolution::Pass,
        keyboard::Key::Named(Named::Escape) => {
            CaptureResolution::Publish(KeyCaptureEvent::Cancelled)
        }
        keyboard::Key::Named(Named::Backspace | Named::Delete) => {
            CaptureResolution::Publish(KeyCaptureEvent::Cleared)
        }
        keyboard::Key::Named(
            Named::Alt
            | Named::AltGraph
            | Named::Control
            | Named::Shift
            | Named::Meta
            | Named::Hyper
            | Named::Super,
        ) => CaptureResolution::Pending,
        _ => KeyStroke::from_iced(key, *modifiers)
            .map(KeyCaptureEvent::Captured)
            .map(CaptureResolution::Publish)
            .unwrap_or(CaptureResolution::Pending),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyA),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn captures_complete_shortcuts_and_keeps_modifiers_pending() {
        assert_eq!(
            resolve_capture_event(&key_event(
                keyboard::Key::Named(keyboard::key::Named::Control),
                keyboard::Modifiers::CTRL,
            )),
            CaptureResolution::Pending
        );
        assert_eq!(
            resolve_capture_event(&key_event(
                keyboard::Key::Character("L".into()),
                keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT,
            )),
            CaptureResolution::Publish(KeyCaptureEvent::Captured(KeyStroke::new(
                "L",
                crate::KeyModifiers {
                    control: true,
                    alt: false,
                    shift: true,
                    logo: false,
                },
            )))
        );
    }

    #[test]
    fn escape_cancels_delete_clears_and_tab_keeps_focus_navigation() {
        assert_eq!(
            resolve_capture_event(&key_event(
                keyboard::Key::Named(keyboard::key::Named::Escape),
                keyboard::Modifiers::empty(),
            )),
            CaptureResolution::Publish(KeyCaptureEvent::Cancelled)
        );
        assert_eq!(
            resolve_capture_event(&key_event(
                keyboard::Key::Named(keyboard::key::Named::Delete),
                keyboard::Modifiers::empty(),
            )),
            CaptureResolution::Publish(KeyCaptureEvent::Cleared)
        );
        assert_eq!(
            resolve_capture_event(&key_event(
                keyboard::Key::Named(keyboard::key::Named::Tab),
                keyboard::Modifiers::empty(),
            )),
            CaptureResolution::Pass
        );
    }
}
