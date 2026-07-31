use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::widget::{button, container};
use iced::{
    Border, Element, Event, Length, Point, Rectangle, Shadow, Size, Theme, Vector, keyboard, touch,
};

use crate::theme::ThemeTokens;
use crate::widgets::menu_surface_style;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PopoverPlacement {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

/// An interactive anchored overlay with keyboard trigger and dismiss behavior.
pub struct Popover<'a, Message> {
    trigger: Element<'a, Message>,
    content: Element<'a, Message>,
    open: bool,
    on_toggle: Message,
    on_close: Message,
    placement: PopoverPlacement,
    gap: f32,
    width: f32,
    close_on_escape: bool,
    close_on_outside: bool,
    tokens: ThemeTokens,
}

impl<'a, Message> Popover<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        trigger: impl Into<Element<'a, Message>>,
        content: impl Into<Element<'a, Message>>,
        open: bool,
        on_toggle: Message,
        on_close: Message,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            trigger: trigger.into(),
            content: content.into(),
            open,
            on_toggle,
            on_close,
            placement: PopoverPlacement::Bottom,
            gap: 6.0,
            width: 240.0,
            close_on_escape: true,
            close_on_outside: true,
            tokens: theme.into(),
        }
    }

    pub fn placement(mut self, placement: PopoverPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(120.0);
        self
    }

    pub fn close_on_escape(mut self, enabled: bool) -> Self {
        self.close_on_escape = enabled;
        self
    }

    pub fn close_on_outside(mut self, enabled: bool) -> Self {
        self.close_on_outside = enabled;
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        let trigger_text = self.tokens.colors.text;
        let trigger = button(self.trigger)
            .padding(0)
            .on_press(self.on_toggle)
            .style(move |_theme, _status| button::Style {
                background: None,
                text_color: trigger_text,
                border: Border::default(),
                shadow: Shadow::default(),
                snap: true,
            });
        let surface = container(self.content)
            .width(Length::Fixed(self.width))
            .padding(10)
            .style(menu_surface_style(self.tokens));
        Element::new(PopoverWidget {
            trigger: trigger.into(),
            surface: surface.into(),
            open: self.open,
            on_close: self.on_close,
            placement: self.placement,
            gap: self.gap,
            close_on_escape: self.close_on_escape,
            close_on_outside: self.close_on_outside,
        })
    }
}

struct PopoverWidget<'a, Message> {
    trigger: Element<'a, Message>,
    surface: Element<'a, Message>,
    open: bool,
    on_close: Message,
    placement: PopoverPlacement,
    gap: f32,
    close_on_escape: bool,
    close_on_outside: bool,
}

#[derive(Debug, Default)]
struct PopoverState;

impl<Message> Widget<Message, Theme, iced::Renderer> for PopoverWidget<'_, Message>
where
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<PopoverState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(PopoverState)
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(&mut [self.trigger.as_widget_mut(), self.surface.as_widget_mut()]);
    }

    fn size(&self) -> Size<Length> {
        self.trigger.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.trigger
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
        self.trigger.as_widget_mut().update(
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
        self.trigger.as_widget().draw(
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
        self.trigger.as_widget().mouse_interaction(
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
        self.trigger
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
        let mut children = tree.children.iter_mut();
        let trigger_overlay = self.trigger.as_widget_mut().overlay(
            children.next().expect("popover trigger state"),
            layout,
            renderer,
            viewport,
            translation,
        );
        let surface_tree = children.next().expect("popover surface state");
        let popover = self.open.then(|| {
            overlay::Element::new(Box::new(PopoverOverlay {
                trigger_bounds: layout.bounds() + translation,
                surface: &mut self.surface,
                tree: surface_tree,
                on_close: self.on_close.clone(),
                placement: self.placement,
                gap: self.gap,
                close_on_escape: self.close_on_escape,
                close_on_outside: self.close_on_outside,
            }))
        });
        let overlays: Vec<_> = trigger_overlay.into_iter().chain(popover).collect();
        (!overlays.is_empty()).then(|| overlay::Group::with_children(overlays).overlay())
    }
}

struct PopoverOverlay<'a, 'b, Message> {
    trigger_bounds: Rectangle,
    surface: &'b mut Element<'a, Message>,
    tree: &'b mut widget::Tree,
    on_close: Message,
    placement: PopoverPlacement,
    gap: f32,
    close_on_escape: bool,
    close_on_outside: bool,
}

impl<Message> overlay::Overlay<Message, Theme, iced::Renderer> for PopoverOverlay<'_, '_, Message>
where
    Message: Clone,
{
    fn layout(&mut self, renderer: &iced::Renderer, bounds: Size) -> layout::Node {
        let surface = self.surface.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let size = surface.size();
        let point =
            resolve_popover_position(self.trigger_bounds, size, bounds, self.placement, self.gap);
        layout::Node::with_children(size, vec![surface]).move_to(point)
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        let escape = matches!(
            event,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            })
        );
        let outside_press = matches!(
            event,
            Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
                | Event::Touch(touch::Event::FingerPressed { .. })
        ) && !cursor.is_over(layout.bounds());
        if (escape && self.close_on_escape) || (outside_press && self.close_on_outside) {
            shell.publish(self.on_close.clone());
            shell.capture_event();
            return;
        }
        self.surface.as_widget_mut().update(
            self.tree,
            event,
            layout.children().next().expect("popover surface layout"),
            cursor,
            renderer,
            shell,
            &Rectangle::with_size(Size::INFINITE),
        );
    }

    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.surface.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout.children().next().expect("popover surface layout"),
            cursor,
            &Rectangle::with_size(Size::INFINITE),
        );
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.surface.as_widget_mut().operate(
            self.tree,
            layout.children().next().expect("popover surface layout"),
            renderer,
            operation,
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.surface.as_widget().mouse_interaction(
            self.tree,
            layout.children().next().expect("popover surface layout"),
            cursor,
            &Rectangle::with_size(Size::INFINITE),
            renderer,
        )
    }

    fn overlay<'c>(
        &'c mut self,
        layout: Layout<'c>,
        renderer: &iced::Renderer,
    ) -> Option<overlay::Element<'c, Message, Theme, iced::Renderer>> {
        self.surface.as_widget_mut().overlay(
            self.tree,
            layout.children().next().expect("popover surface layout"),
            renderer,
            &Rectangle::with_size(Size::INFINITE),
            Vector::ZERO,
        )
    }
}

fn resolve_popover_position(
    trigger: Rectangle,
    surface: Size,
    viewport: Size,
    placement: PopoverPlacement,
    gap: f32,
) -> Point {
    let mut point = match placement {
        PopoverPlacement::Top => Point::new(
            trigger.center_x() - surface.width / 2.0,
            trigger.y - surface.height - gap,
        ),
        PopoverPlacement::Bottom => Point::new(
            trigger.center_x() - surface.width / 2.0,
            trigger.y + trigger.height + gap,
        ),
        PopoverPlacement::Left => Point::new(
            trigger.x - surface.width - gap,
            trigger.center_y() - surface.height / 2.0,
        ),
        PopoverPlacement::Right => Point::new(
            trigger.x + trigger.width + gap,
            trigger.center_y() - surface.height / 2.0,
        ),
    };
    point.x = point
        .x
        .clamp(0.0, (viewport.width - surface.width).max(0.0));
    point.y = point
        .y
        .clamp(0.0, (viewport.height - surface.height).max(0.0));
    point
}

#[cfg(test)]
mod tests {
    use super::{PopoverPlacement, resolve_popover_position};
    use iced::{Point, Rectangle, Size};

    #[test]
    fn popover_placement_is_anchored_and_clamped_to_the_viewport() {
        assert_eq!(
            resolve_popover_position(
                Rectangle::new(Point::new(90.0, 80.0), Size::new(20.0, 20.0)),
                Size::new(80.0, 60.0),
                Size::new(120.0, 120.0),
                PopoverPlacement::Bottom,
                6.0,
            ),
            Point::new(40.0, 60.0)
        );
        assert_eq!(
            resolve_popover_position(
                Rectangle::new(Point::new(4.0, 4.0), Size::new(20.0, 20.0)),
                Size::new(80.0, 60.0),
                Size::new(120.0, 120.0),
                PopoverPlacement::Left,
                6.0,
            ),
            Point::new(0.0, 0.0)
        );
    }
}
