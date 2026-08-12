use std::rc::Rc;

use iced::advanced::Renderer as _;
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::widget::column;
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Theme, touch};

const REORDER_DRAG_THRESHOLD: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReorderSource {
    Mouse,
    Touch(touch::Finger),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ReorderOutcome {
    Captured,
    Select(usize),
    Reorder {
        source: usize,
        before: Option<usize>,
    },
    Cancelled,
}

#[derive(Debug, Default)]
struct ReorderState {
    source: Option<ReorderSource>,
    active_index: Option<usize>,
    start_position: Option<Point>,
    position: Option<Point>,
    moved: bool,
}

impl ReorderState {
    fn update(
        &mut self,
        event: &Event,
        bounds: &[Rectangle],
        drag_sources: &[bool],
        drop_targets: &[bool],
        cursor: mouse::Cursor,
    ) -> Option<ReorderOutcome> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.source.is_none() =>
            {
                let position = cursor.position()?;
                let index = item_at(bounds, drag_sources, position)?;
                self.begin(ReorderSource::Mouse, index, position);
                Some(ReorderOutcome::Captured)
            }
            Event::Mouse(mouse::Event::CursorMoved { position })
                if self.source == Some(ReorderSource::Mouse) =>
            {
                self.move_to(*position);
                Some(ReorderOutcome::Captured)
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if self.source == Some(ReorderSource::Mouse) =>
            {
                Some(self.finish(bounds, drop_targets))
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) if self.source.is_none() => {
                let index = item_at(bounds, drag_sources, *position)?;
                self.begin(ReorderSource::Touch(*id), index, *position);
                Some(ReorderOutcome::Captured)
            }
            Event::Touch(touch::Event::FingerMoved { id, position })
                if self.source == Some(ReorderSource::Touch(*id)) =>
            {
                self.move_to(*position);
                Some(ReorderOutcome::Captured)
            }
            Event::Touch(touch::Event::FingerLifted { id, position })
                if self.source == Some(ReorderSource::Touch(*id)) =>
            {
                self.move_to(*position);
                Some(self.finish(bounds, drop_targets))
            }
            Event::Touch(touch::Event::FingerLost { id, .. })
                if self.source == Some(ReorderSource::Touch(*id)) =>
            {
                self.clear();
                Some(ReorderOutcome::Cancelled)
            }
            Event::Window(iced::window::Event::Unfocused) if self.source.is_some() => {
                self.clear();
                Some(ReorderOutcome::Cancelled)
            }
            _ => None,
        }
    }

    fn begin(&mut self, source: ReorderSource, index: usize, position: Point) {
        self.source = Some(source);
        self.active_index = Some(index);
        self.start_position = Some(position);
        self.position = Some(position);
        self.moved = false;
    }

    fn move_to(&mut self, position: Point) {
        self.position = Some(position);
        if let Some(start) = self.start_position {
            let delta = position - start;
            self.moved |= delta.x * delta.x + delta.y * delta.y
                >= REORDER_DRAG_THRESHOLD * REORDER_DRAG_THRESHOLD;
        }
    }

    fn finish(&mut self, bounds: &[Rectangle], drop_targets: &[bool]) -> ReorderOutcome {
        let source = self.active_index.expect("active list drag has a source");
        let outcome = if self.moved {
            ReorderOutcome::Reorder {
                source,
                before: drop_before_index(bounds, drop_targets, Some(source), self.position),
            }
        } else {
            ReorderOutcome::Select(source)
        };
        self.clear();
        outcome
    }

    fn clear(&mut self) {
        self.source = None;
        self.active_index = None;
        self.start_position = None;
        self.position = None;
        self.moved = false;
    }
}

fn item_at(bounds: &[Rectangle], enabled: &[bool], position: Point) -> Option<usize> {
    bounds
        .iter()
        .enumerate()
        .find(|(index, bounds)| {
            enabled.get(*index).copied().unwrap_or(false) && bounds.contains(position)
        })
        .map(|(index, _)| index)
}

fn drop_before_index(
    bounds: &[Rectangle],
    drop_targets: &[bool],
    excluded: Option<usize>,
    position: Option<Point>,
) -> Option<usize> {
    let position = position?;
    bounds
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            Some(*index) != excluded && drop_targets.get(*index).copied().unwrap_or(false)
        })
        .find(|(_, bounds)| position.y < bounds.center_y())
        .map(|(index, _)| index)
}

fn reorder_changes_position(length: usize, source: usize, before: Option<usize>) -> bool {
    if source >= length || before == Some(source) {
        return false;
    }
    let mut reordered = (0..length)
        .filter(|index| *index != source)
        .collect::<Vec<_>>();
    let insert_at = before
        .and_then(|before| reordered.iter().position(|index| *index == before))
        .unwrap_or(reordered.len());
    reordered.insert(insert_at, source);
    reordered.into_iter().ne(0..length)
}

/// One passive row in a [`ReorderList`].
///
/// The list owns pointer selection and drag gestures. Interactive controls that need independent
/// pointer handling should remain outside the row.
pub struct ReorderItem<'a, T, Message> {
    value: T,
    content: Element<'a, Message>,
    draggable: bool,
    drop_target: bool,
}

impl<'a, T, Message> ReorderItem<'a, T, Message> {
    pub fn new(value: T, content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            value,
            content: content.into(),
            draggable: true,
            drop_target: true,
        }
    }

    pub fn draggable(mut self, draggable: bool) -> Self {
        self.draggable = draggable;
        self.drop_target = draggable;
        self
    }

    /// Configure whether this row can receive a drop.
    ///
    /// This allows passive destination rows with `.draggable(false).drop_target(true)` while
    /// preserving the existing behavior where disabled rows are neither sources nor targets.
    pub fn drop_target(mut self, drop_target: bool) -> Self {
        self.drop_target = drop_target;
        self
    }
}

type SelectHandler<'a, T, Message> = Rc<dyn Fn(T) -> Message + 'a>;
type ReorderHandler<'a, T, Message> = Rc<dyn Fn(T, Option<T>) -> Message + 'a>;

/// A vertical list with thresholded mouse/touch reordering.
///
/// Reorder messages use the moved value and the value that should follow the move. `None` means
/// the item was dropped at the end. NanaUI does not mutate or persist the application-owned list.
pub struct ReorderList<'a, T, Message> {
    items: Vec<ReorderItem<'a, T, Message>>,
    on_select: SelectHandler<'a, T, Message>,
    on_reorder: Option<ReorderHandler<'a, T, Message>>,
    spacing: f32,
    indicator: Color,
}

impl<'a, T, Message> ReorderList<'a, T, Message>
where
    T: Clone + 'a,
    Message: Clone + 'a,
{
    pub fn new(
        items: impl IntoIterator<Item = ReorderItem<'a, T, Message>>,
        on_select: impl Fn(T) -> Message + 'a,
    ) -> Self {
        Self {
            items: items.into_iter().collect(),
            on_select: Rc::new(on_select),
            on_reorder: None,
            spacing: 1.0,
            indicator: Color::from_rgb8(62, 143, 255),
        }
    }

    pub fn on_reorder(mut self, on_reorder: impl Fn(T, Option<T>) -> Message + 'a) -> Self {
        self.on_reorder = Some(Rc::new(on_reorder));
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing.max(0.0);
        self
    }

    pub fn indicator(mut self, color: Color) -> Self {
        self.indicator = color;
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        let values = self
            .items
            .iter()
            .map(|item| item.value.clone())
            .collect::<Vec<_>>();
        let drag_sources = self
            .items
            .iter()
            .map(|item| item.draggable)
            .collect::<Vec<_>>();
        let drop_targets = self
            .items
            .iter()
            .map(|item| item.drop_target)
            .collect::<Vec<_>>();
        let mut content = column![].spacing(self.spacing).width(Length::Fill);
        for item in self.items {
            content = content.push(item.content);
        }
        Element::new(ReorderListWidget {
            content: content.into(),
            values,
            drag_sources,
            drop_targets,
            on_select: self.on_select,
            on_reorder: self.on_reorder,
            indicator: self.indicator,
        })
    }
}

struct ReorderListWidget<'a, T, Message> {
    content: Element<'a, Message>,
    values: Vec<T>,
    drag_sources: Vec<bool>,
    drop_targets: Vec<bool>,
    on_select: SelectHandler<'a, T, Message>,
    on_reorder: Option<ReorderHandler<'a, T, Message>>,
    indicator: Color,
}

impl<T, Message> Widget<Message, Theme, iced::Renderer> for ReorderListWidget<'_, T, Message>
where
    T: Clone,
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<ReorderState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(ReorderState::default())
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(&mut [self.content.as_widget_mut()]);
        let state = tree.state.downcast_mut::<ReorderState>();
        if state
            .active_index
            .is_some_and(|index| index >= self.values.len())
        {
            state.clear();
        }
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
        layout::Node::with_children(content.size(), vec![content])
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
        let content_layout = layout.children().next().expect("reorder list content");
        let bounds = content_layout
            .children()
            .map(|item| item.bounds())
            .collect::<Vec<_>>();
        let state = tree.state.downcast_mut::<ReorderState>();
        if let Some(outcome) = state.update(
            event,
            &bounds,
            &self.drag_sources,
            &self.drop_targets,
            cursor,
        ) {
            match outcome {
                ReorderOutcome::Select(index) => {
                    if let Some(value) = self.values.get(index).cloned() {
                        shell.publish((self.on_select)(value));
                    }
                }
                ReorderOutcome::Reorder { source, before } => {
                    if reorder_changes_position(self.values.len(), source, before)
                        && let (Some(value), Some(on_reorder)) =
                            (self.values.get(source).cloned(), self.on_reorder.as_ref())
                    {
                        let before = before.and_then(|index| self.values.get(index).cloned());
                        shell.publish(on_reorder(value, before));
                    }
                }
                ReorderOutcome::Cancelled | ReorderOutcome::Captured => {}
            }
            shell.capture_event();
            shell.request_redraw();
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
        let content_layout = layout.children().next().expect("reorder list content");
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            content_layout,
            cursor,
            viewport,
        );

        let bounds = content_layout
            .children()
            .map(|item| item.bounds())
            .collect::<Vec<_>>();
        let state = tree.state.downcast_ref::<ReorderState>();
        let Some(source) = state.moved.then_some(state.active_index).flatten() else {
            return;
        };
        let before = drop_before_index(&bounds, &self.drop_targets, Some(source), state.position);
        if !reorder_changes_position(bounds.len(), source, before) {
            return;
        }
        let y = before
            .and_then(|index| bounds.get(index).map(|bounds| bounds.y - 2.0))
            .or_else(|| bounds.last().map(|bounds| bounds.y + bounds.height + 2.0));
        let Some(y) = y else {
            return;
        };
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle::new(
                    Point::new(content_layout.bounds().x + 4.0, y),
                    Size::new((content_layout.bounds().width - 8.0).max(0.0), 2.0),
                ),
                ..renderer::Quad::default()
            },
            self.indicator,
        );
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let content_layout = layout.children().next().expect("reorder list content");
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
        let content_layout = layout.children().next().expect("reorder list content");
        let state = tree.state.downcast_ref::<ReorderState>();
        if state.source.is_some() {
            mouse::Interaction::Grabbing
        } else {
            let bounds = content_layout
                .children()
                .map(|item| item.bounds())
                .collect::<Vec<_>>();
            if cursor
                .position()
                .and_then(|position| item_at(&bounds, &self.drag_sources, position))
                .is_some()
            {
                mouse::Interaction::Grab
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
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        let content_layout = layout.children().next().expect("reorder list content");
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            content_layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_bounds() -> Vec<Rectangle> {
        (0..3)
            .map(|index| {
                Rectangle::new(Point::new(0.0, index as f32 * 29.0), Size::new(180.0, 28.0))
            })
            .collect()
    }

    #[test]
    fn click_selects_without_reordering() {
        let bounds = row_bounds();
        let enabled = vec![true; bounds.len()];
        let mut state = ReorderState::default();
        let point = Point::new(40.0, 42.0);

        assert_eq!(
            state.update(
                &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                &bounds,
                &enabled,
                &enabled,
                mouse::Cursor::Available(point),
            ),
            Some(ReorderOutcome::Captured)
        );
        assert_eq!(
            state.update(
                &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                &bounds,
                &enabled,
                &enabled,
                mouse::Cursor::Available(point),
            ),
            Some(ReorderOutcome::Select(1))
        );
    }

    #[test]
    fn vertical_drag_uses_before_value_contract() {
        let bounds = row_bounds();
        let enabled = vec![true; bounds.len()];
        let mut state = ReorderState::default();
        let start = Point::new(40.0, 12.0);
        let end = Point::new(40.0, 80.0);
        state.update(
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            &bounds,
            &enabled,
            &enabled,
            mouse::Cursor::Available(start),
        );
        state.update(
            &Event::Mouse(mouse::Event::CursorMoved { position: end }),
            &bounds,
            &enabled,
            &enabled,
            mouse::Cursor::Available(end),
        );

        assert_eq!(
            state.update(
                &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                &bounds,
                &enabled,
                &enabled,
                mouse::Cursor::Available(end),
            ),
            Some(ReorderOutcome::Reorder {
                source: 0,
                before: None,
            })
        );
        assert!(reorder_changes_position(3, 0, None));
        assert!(!reorder_changes_position(3, 1, Some(2)));
    }

    #[test]
    fn disabled_rows_are_neither_sources_nor_drop_targets() {
        let bounds = row_bounds();
        let enabled = vec![true, false, true];
        assert_eq!(item_at(&bounds, &enabled, Point::new(40.0, 42.0)), None);
        assert_eq!(
            drop_before_index(&bounds, &enabled, Some(0), Some(Point::new(40.0, 42.0))),
            Some(2)
        );
    }

    #[test]
    fn passive_destination_rows_accept_a_drag_without_becoming_sources() {
        let bounds = row_bounds();
        let drag_sources = vec![true, false, false];
        let drop_targets = vec![false, true, true];
        let mut state = ReorderState::default();
        let start = Point::new(40.0, 12.0);
        let end = Point::new(40.0, 42.0);
        state.update(
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            &bounds,
            &drag_sources,
            &drop_targets,
            mouse::Cursor::Available(start),
        );
        state.update(
            &Event::Mouse(mouse::Event::CursorMoved { position: end }),
            &bounds,
            &drag_sources,
            &drop_targets,
            mouse::Cursor::Available(end),
        );

        assert_eq!(
            state.update(
                &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                &bounds,
                &drag_sources,
                &drop_targets,
                mouse::Cursor::Available(end),
            ),
            Some(ReorderOutcome::Reorder {
                source: 0,
                before: Some(1),
            })
        );
        assert_eq!(item_at(&bounds, &drag_sources, end), None);
    }
}
