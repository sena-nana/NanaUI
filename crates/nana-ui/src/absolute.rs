//! Absolute placement that does **not** shrink available layout space the way
//! iced's [`pin`](iced::widget::pin) does (`available = parent − position`).

use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Layout, mouse, overlay, renderer};
use iced::widget::container;
use iced::{Element, Event, Length, Point, Rectangle, Shell, Size};

/// Fills the parent and places `content` at `position` without edge clamping.
///
/// Use this for anchored overlays (menus, toasts, node chrome) when the child
/// must keep its intrinsic size even near the bottom-right of the viewport.
pub struct Absolute<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Theme: container::Catalog,
    Renderer: iced::advanced::Renderer,
{
    content: Element<'a, Message, Theme, Renderer>,
    position: Point,
}

impl<'a, Message, Theme, Renderer> Absolute<'a, Message, Theme, Renderer>
where
    Theme: container::Catalog,
    Renderer: iced::advanced::Renderer,
{
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

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Absolute<'a, Message, Theme, Renderer>
where
    Theme: container::Catalog,
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::stateless()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::None
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fill, limits.max());
        let content = self.content.as_widget().layout(
            &mut tree.children[0],
            renderer,
            &iced::advanced::layout::Limits::new(Size::ZERO, size),
        );
        iced::advanced::layout::Node::with_children(size, vec![content.move_to(self.position)])
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget().operate(
            &mut tree.children[0],
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
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().expect("absolute content"),
            cursor,
            renderer,
            clipboard,
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
            &tree.children[0],
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
            &tree.children[0],
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
            &mut tree.children[0],
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
    Theme: container::Catalog + 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(widget: Absolute<'a, Message, Theme, Renderer>) -> Self {
        Element::new(widget)
    }
}
