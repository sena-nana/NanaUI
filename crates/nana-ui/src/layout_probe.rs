use std::rc::Rc;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Shell, mouse, overlay};
use iced::{Element, Event, Length, Rectangle, Size};

/// Logical widget bounds relative to the hosted window content area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LayoutBounds {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

impl From<Rectangle> for LayoutBounds {
    fn from(bounds: Rectangle) -> Self {
        Self::new(bounds.x, bounds.y, bounds.width, bounds.height)
    }
}

#[derive(Debug, Default)]
struct LayoutProbeState {
    reported: Option<LayoutBounds>,
}

impl LayoutProbeState {
    fn record(&mut self, bounds: LayoutBounds) -> bool {
        if self.reported == Some(bounds) {
            return false;
        }
        self.reported = Some(bounds);
        true
    }
}

/// Wraps content and emits a message whenever its resolved layout bounds change.
///
/// Bounds use the same logical, window-relative coordinate space as Iced layout.
/// The probe does not own visibility or interpret the measured region; consumers
/// decide how to apply the result to hosted content such as a child WebView.
pub struct LayoutProbe<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_bounds: Rc<dyn Fn(LayoutBounds) -> Message + 'a>,
}

impl<'a, Message, Theme, Renderer> LayoutProbe<'a, Message, Theme, Renderer> {
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        on_bounds: impl Fn(LayoutBounds) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            on_bounds: Rc::new(on_bounds),
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for LayoutProbe<'_, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<LayoutProbeState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(LayoutProbeState::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [self.content.as_widget_mut()]);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let content = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(content.size(), vec![content])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().expect("layout probe content"),
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
        let bounds = LayoutBounds::from(layout.bounds());
        if tree.state.downcast_mut::<LayoutProbeState>().record(bounds) {
            shell.publish((self.on_bounds)(bounds));
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().expect("layout probe content"),
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
            &tree.children[0],
            layout.children().next().expect("layout probe content"),
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
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().expect("layout probe content"),
            cursor,
            viewport,
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
            layout.children().next().expect("layout probe content"),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<LayoutProbe<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(probe: LayoutProbe<'a, Message, Theme, Renderer>) -> Self {
        Element::new(probe)
    }
}

#[cfg(test)]
mod tests {
    use super::{LayoutBounds, LayoutProbeState};

    #[test]
    fn layout_probe_reports_initial_and_changed_bounds_once() {
        let mut state = LayoutProbeState::default();
        let initial = LayoutBounds::new(24.0, 36.0, 320.0, 640.0);
        assert!(state.record(initial));
        assert!(!state.record(initial));

        let resized = LayoutBounds::new(24.0, 36.0, 480.0, 640.0);
        assert!(state.record(resized));
        assert!(!state.record(resized));
    }
}
