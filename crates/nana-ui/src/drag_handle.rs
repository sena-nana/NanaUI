use std::rc::Rc;

use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::{Element, Event, Length, Point, Rectangle, Size, Theme, Vector, touch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragSource {
    Mouse,
    Touch(touch::Finger),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragSignal {
    Start,
    Move(Point),
    End,
    Reset,
    Hover(bool),
}

#[derive(Debug, Default)]
struct DragHandleState {
    source: Option<DragSource>,
    hovered: bool,
    cursor_position: Option<Point>,
    previous_click: Option<mouse::Click>,
}

impl DragHandleState {
    fn signals(
        &mut self,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<DragSignal> {
        let mut signals = Vec::new();
        let position = match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                self.cursor_position = Some(*position);
                Some(*position)
            }
            Event::Mouse(mouse::Event::CursorLeft)
            | Event::Window(iced::window::Event::Unfocused) => {
                self.cursor_position = None;
                None
            }
            _ => self.cursor_position.or_else(|| cursor.position()),
        };
        let hovered = position.is_some_and(|position| bounds.contains(position));
        if self.hovered != hovered {
            self.hovered = hovered;
            signals.push(DragSignal::Hover(hovered));
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if hovered => {
                let Some(position) = position else {
                    return signals;
                };
                self.source = Some(DragSource::Mouse);
                signals.push(DragSignal::Start);
                signals.push(DragSignal::Move(position));

                let click = mouse::Click::new(position, mouse::Button::Left, self.previous_click);
                if click.kind() == mouse::click::Kind::Double {
                    signals.push(DragSignal::Reset);
                }
                self.previous_click = Some(click);
            }
            Event::Mouse(mouse::Event::CursorMoved { position })
                if self.source == Some(DragSource::Mouse) =>
            {
                signals.push(DragSignal::Move(*position));
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if self.source == Some(DragSource::Mouse) =>
            {
                self.source = None;
                signals.push(DragSignal::End);
            }
            Event::Touch(touch::Event::FingerPressed { id, position })
                if bounds.contains(*position) =>
            {
                self.source = Some(DragSource::Touch(*id));
                signals.push(DragSignal::Start);
                signals.push(DragSignal::Move(*position));
            }
            Event::Touch(touch::Event::FingerMoved { id, position })
                if self.source == Some(DragSource::Touch(*id)) =>
            {
                signals.push(DragSignal::Move(*position));
            }
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) if self.source == Some(DragSource::Touch(*id)) => {
                self.source = None;
                signals.push(DragSignal::End);
            }
            Event::Window(iced::window::Event::Unfocused) if self.source.is_some() => {
                self.source = None;
                signals.push(DragSignal::End);
            }
            _ => {}
        }
        signals
    }
}

pub(crate) struct DragHandle<'a, Message> {
    content: Element<'a, Message>,
    translation: Vector,
    on_start: Message,
    on_move: Rc<dyn Fn(Point) -> Message + 'a>,
    on_end: Message,
    on_reset: Message,
    on_hover: Rc<dyn Fn(bool) -> Message + 'a>,
    interaction: mouse::Interaction,
}

impl<'a, Message> DragHandle<'a, Message> {
    pub(crate) fn new(
        content: impl Into<Element<'a, Message>>,
        on_start: Message,
        on_move: impl Fn(Point) -> Message + 'a,
        on_end: Message,
        on_reset: Message,
        on_hover: impl Fn(bool) -> Message + 'a,
        interaction: mouse::Interaction,
    ) -> Self {
        Self {
            content: content.into(),
            translation: Vector::ZERO,
            on_start,
            on_move: Rc::new(on_move),
            on_end,
            on_reset,
            on_hover: Rc::new(on_hover),
            interaction,
        }
    }

    pub(crate) fn translate(mut self, translation: Vector) -> Self {
        self.translation = translation;
        self
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for DragHandle<'_, Message>
where
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<DragHandleState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(DragHandleState::default())
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
        let content = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        translated_layout(content, self.translation)
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
        let content_layout = layout.children().next().expect("drag handle content");
        let signals = tree.state.downcast_mut::<DragHandleState>().signals(
            event,
            content_layout.bounds(),
            cursor,
        );
        if !signals.is_empty() {
            for signal in signals {
                shell.publish(match signal {
                    DragSignal::Start => self.on_start.clone(),
                    DragSignal::Move(position) => (self.on_move)(position),
                    DragSignal::End => self.on_end.clone(),
                    DragSignal::Reset => self.on_reset.clone(),
                    DragSignal::Hover(hovered) => (self.on_hover)(hovered),
                });
            }
            shell.capture_event();
            return;
        }

        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            content_layout,
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
        let content_layout = layout.children().next().expect("drag handle content");
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            content_layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let content_layout = layout.children().next().expect("drag handle content");
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            content_layout,
            renderer,
            operation,
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
        let content_layout = layout.children().next().expect("drag handle content");
        let state = tree.state.downcast_ref::<DragHandleState>();
        if state.source.is_some() || cursor.is_over(content_layout.bounds()) {
            self.interaction
        } else {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                content_layout,
                cursor,
                viewport,
                renderer,
            )
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        let content_layout = layout.children().next().expect("drag handle content");
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            content_layout,
            renderer,
            viewport,
            translation,
        )
    }
}

fn translated_layout(content: layout::Node, translation: Vector) -> layout::Node {
    let size = content.size();
    layout::Node::with_children(size, vec![content.translate(translation)])
}

impl<'a, Message> From<DragHandle<'a, Message>> for Element<'a, Message>
where
    Message: Clone + 'a,
{
    fn from(handle: DragHandle<'a, Message>) -> Self {
        Element::new(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_moves_the_hit_content_without_consuming_layout_space() {
        let layout = translated_layout(
            layout::Node::new(Size::new(8.0, 100.0)),
            Vector::new(4.0, 0.0),
        );

        assert_eq!(layout.size(), Size::new(8.0, 100.0));
        assert_eq!(
            layout.children()[0].bounds(),
            Rectangle::new(Point::new(4.0, 0.0), Size::new(8.0, 100.0))
        );
    }

    #[test]
    fn mouse_drag_keeps_moving_and_ends_outside_the_handle() {
        let bounds = Rectangle::new(Point::new(100.0, 40.0), Size::new(8.0, 200.0));
        let mut state = DragHandleState::default();

        let press = Point::new(104.0, 90.0);
        assert_eq!(
            state.signals(
                &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Available(press),
            ),
            vec![
                DragSignal::Hover(true),
                DragSignal::Start,
                DragSignal::Move(press)
            ]
        );

        let outside = Point::new(164.0, 90.0);
        assert_eq!(
            state.signals(
                &Event::Mouse(mouse::Event::CursorMoved { position: outside }),
                bounds,
                mouse::Cursor::Available(outside),
            ),
            vec![DragSignal::Hover(false), DragSignal::Move(outside)]
        );
        assert_eq!(
            state.signals(
                &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Available(outside),
            ),
            vec![DragSignal::End]
        );
        assert!(state.source.is_none());
    }

    #[test]
    fn queued_press_uses_the_preceding_cursor_event_instead_of_the_batch_cursor() {
        let bounds = Rectangle::new(Point::new(100.0, 40.0), Size::new(8.0, 200.0));
        let mut state = DragHandleState::default();
        let press = Point::new(104.0, 90.0);
        let outside = Point::new(164.0, 90.0);

        assert_eq!(
            state.signals(
                &Event::Mouse(mouse::Event::CursorMoved { position: press }),
                bounds,
                mouse::Cursor::Available(outside),
            ),
            vec![DragSignal::Hover(true)]
        );
        assert_eq!(
            state.signals(
                &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Available(outside),
            ),
            vec![DragSignal::Start, DragSignal::Move(press)]
        );
        assert_eq!(
            state.signals(
                &Event::Mouse(mouse::Event::CursorMoved { position: outside }),
                bounds,
                mouse::Cursor::Available(outside),
            ),
            vec![DragSignal::Hover(false), DragSignal::Move(outside)]
        );
    }

    #[test]
    fn window_unfocus_cancels_an_active_drag() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(8.0, 100.0));
        let mut state = DragHandleState::default();
        let press = Point::new(4.0, 20.0);
        state.signals(
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(press),
        );

        assert_eq!(
            state.signals(
                &Event::Window(iced::window::Event::Unfocused),
                bounds,
                mouse::Cursor::Unavailable,
            ),
            vec![DragSignal::Hover(false), DragSignal::End]
        );
        assert!(state.source.is_none());
    }
}
