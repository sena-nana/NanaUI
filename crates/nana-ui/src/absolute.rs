//! Absolute placement that does **not** shrink available layout space the way
//! iced's [`pin`](iced::widget::pin) does (`available = parent − position`).

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Shell, mouse, overlay};
use iced::{Element, Event, Length, Point, Rectangle, Size};

/// Fills the parent and places `content` at `position` without edge clamping.
///
/// Use this for anchored overlays (menus, toasts, node chrome) when the child
/// must keep its intrinsic size even near the bottom-right of the viewport.
pub struct Absolute<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    position: Point,
}

impl<'a, Message, Theme, Renderer> Absolute<'a, Message, Theme, Renderer> {
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        position: Point,
    ) -> Self {
        Self {
            content: content.into(),
            position,
        }
    }
}

/// Max size offered to absolutely positioned content.
///
/// iced [`pin`](iced::widget::pin) uses `parent − position`, which clamps
/// `Length::Fixed` children when the anchor is near the bottom-right and makes
/// menu lists lay out at ~0 height. Absolute always offers the full parent.
pub fn absolute_content_max(parent: Size, _position: Point) -> Size {
    parent
}

/// What iced `pin` would offer at the same anchor — kept for regression tests.
fn pin_content_max(parent: Size, position: Point) -> Size {
    Size::new(
        (parent.width - position.x).max(0.0),
        (parent.height - position.y).max(0.0),
    )
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Absolute<'a, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn diff(&mut self, tree: &mut Tree) {
        self.content.as_widget_mut().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fill, limits.max());
        let content_max = absolute_content_max(size, self.position);
        let node = self
            .content
            .as_widget_mut()
            .layout(
                tree,
                renderer,
                &layout::Limits::new(Size::ZERO, content_max),
            )
            .move_to(self.position);
        layout::Node::with_children(size, vec![node])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(
            tree,
            layout.children().next().expect("absolute content"),
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            tree,
            event,
            layout.children().next().expect("absolute content"),
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            tree,
            layout.children().next().expect("absolute content"),
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(clipped) = layout.bounds().intersection(viewport) else {
            return;
        };
        self.content.as_widget().draw(
            tree,
            renderer,
            theme,
            style,
            layout.children().next().expect("absolute content"),
            cursor,
            &clipped,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            tree,
            layout.children().next().expect("absolute content"),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<Absolute<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(widget: Absolute<'a, Message, Theme, Renderer>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::{absolute_content_max, pin_content_max};
    use iced::{Point, Size};

    #[test]
    fn absolute_keeps_full_parent_when_pin_would_clamp_a_corner_menu() {
        let parent = Size::new(800.0, 600.0);
        let position = Point::new(760.0, 560.0);
        let menu = Size::new(200.0, 240.0);

        let pin_max = pin_content_max(parent, position);
        let absolute_max = absolute_content_max(parent, position);

        assert!(
            pin_max.height < menu.height,
            "pin available height must be the regression case"
        );
        assert_eq!(absolute_max, parent);
        assert!(
            menu.height.min(absolute_max.height) >= menu.height,
            "Absolute must not clamp Fixed menu height the way pin does"
        );
        assert!(
            menu.height.min(pin_max.height) < menu.height,
            "pin Fixed clamp is the empty-list failure mode"
        );
    }
}
