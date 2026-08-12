use std::rc::Rc;

use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector, keyboard};

use crate::{ActionId, ActionRegistry, KeyContext, KeyStroke, Keymap, KeymapMatch, KeymapState};

/// Captures only shortcuts resolved by an application keymap and forwards every
/// other keyboard event to the wrapped content.
pub struct KeymapLayer<'a, Message> {
    content: Element<'a, Message>,
    keymap: Keymap,
    context: KeyContext,
    registry: ActionRegistry,
    on_action: Rc<dyn Fn(ActionId) -> Message + 'a>,
}

impl<'a, Message> KeymapLayer<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        keymap: Keymap,
        context: KeyContext,
        registry: ActionRegistry,
        on_action: impl Fn(ActionId) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            keymap,
            context,
            registry,
            on_action: Rc::new(on_action),
        }
    }

    pub fn view(self) -> Element<'a, Message> {
        self.into()
    }
}

#[derive(Debug, Default)]
struct KeymapLayerState {
    keymap: KeymapState,
}

impl<Message> Widget<Message, Theme, iced::Renderer> for KeymapLayer<'_, Message>
where
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<KeymapLayerState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(KeymapLayerState::default())
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(&mut [self.content.as_widget_mut()]);
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
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
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
        let state = tree.state.downcast_mut::<KeymapLayerState>();
        match resolve_event(
            event,
            &self.keymap,
            &mut state.keymap,
            &self.context,
            &self.registry,
        ) {
            KeymapMatch::Dispatch(action) => {
                shell.publish((self.on_action)(action));
                shell.capture_event();
                return;
            }
            KeymapMatch::Pending => {
                shell.capture_event();
                return;
            }
            KeymapMatch::NoMatch => {}
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
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
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
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
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

fn resolve_event(
    event: &Event,
    keymap: &Keymap,
    state: &mut KeymapState,
    context: &KeyContext,
    registry: &ActionRegistry,
) -> KeymapMatch {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
        return KeymapMatch::NoMatch;
    };
    let Some(stroke) = KeyStroke::from_iced(key, *modifiers) else {
        return KeymapMatch::NoMatch;
    };
    keymap.resolve(state, stroke, context, registry)
}

impl<'a, Message> From<KeymapLayer<'a, Message>> for Element<'a, Message>
where
    Message: Clone + 'a,
{
    fn from(layer: KeymapLayer<'a, Message>) -> Self {
        Element::new(layer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionDescriptor, KeyBinding, KeyModifiers};

    fn registry() -> ActionRegistry {
        let mut registry = ActionRegistry::new();
        registry
            .register(ActionDescriptor::new(
                "workspace.palette",
                "Command Palette",
            ))
            .expect("action is valid");
        registry
    }

    #[test]
    fn resolves_bound_shortcut_and_leaves_unbound_text_alone() {
        let keymap = Keymap::new([KeyBinding::new(
            "workspace.palette",
            KeyStroke::new("p", KeyModifiers::primary().with_shift()),
        )]);
        let registry = registry();
        let context = KeyContext::new(["workspace"]);
        let mut state = KeymapState::default();
        let primary = if cfg!(target_os = "macos") {
            keyboard::Modifiers::LOGO
        } else {
            keyboard::Modifiers::CTRL
        };
        let shortcut = Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character("P".into()),
            modified_key: keyboard::Key::Character("P".into()),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyP),
            location: keyboard::Location::Standard,
            modifiers: primary | keyboard::Modifiers::SHIFT,
            text: Some("P".into()),
            repeat: false,
        });
        assert_eq!(
            resolve_event(&shortcut, &keymap, &mut state, &context, &registry),
            KeymapMatch::Dispatch(ActionId::new("workspace.palette"))
        );

        let text = Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character("x".into()),
            modified_key: keyboard::Key::Character("x".into()),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyX),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: Some("x".into()),
            repeat: false,
        });
        assert_eq!(
            resolve_event(&text, &keymap, &mut state, &context, &registry),
            KeymapMatch::NoMatch
        );
    }
}
