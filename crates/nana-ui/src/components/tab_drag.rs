use std::rc::Rc;

use iced::advanced::Renderer as _;
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Theme, Vector, touch};
use nana_ui_core::{LogicalPoint, TabDragRect, TabDropIndicator, TabStripPaint};

pub use nana_ui_core::{TabDragGroup, TabDragLease, TabDragSurface};

const TAB_DRAG_THRESHOLD: f32 = 4.0;

fn drag_point(point: Point) -> LogicalPoint {
    LogicalPoint::new(point.x, point.y)
}

fn drag_rect(bounds: Rectangle) -> TabDragRect {
    TabDragRect::new(bounds.x, bounds.y, bounds.width, bounds.height)
}

fn strip_paint<T: Clone>(
    bounds: Rectangle,
    tab_bounds: &[Rectangle],
    values: &[T],
    disabled: &[bool],
    accepts_external_drop: bool,
) -> TabStripPaint<T> {
    TabStripPaint {
        bounds: drag_rect(bounds),
        tab_bounds: tab_bounds.iter().copied().map(drag_rect).collect(),
        values: values.to_vec(),
        disabled: disabled.to_vec(),
        accepts_external_drop,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabDragSource {
    Mouse,
    Touch(touch::Finger),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TabDragOutcome {
    Captured,
    Select(usize),
    Reorder {
        source: usize,
        before: Option<usize>,
        position: Point,
    },
    Cancelled,
}

#[derive(Debug, Default)]
struct TabDragState {
    source: Option<TabDragSource>,
    active_index: Option<usize>,
    start_position: Option<Point>,
    position: Option<Point>,
    moved: bool,
}

impl TabDragState {
    fn update(
        &mut self,
        event: &Event,
        bounds: &[Rectangle],
        disabled: &[bool],
        cursor: mouse::Cursor,
    ) -> Option<TabDragOutcome> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.source.is_none() =>
            {
                let position = cursor.position()?;
                let index = tab_at(bounds, disabled, position)?;
                self.begin(TabDragSource::Mouse, index, position);
                Some(TabDragOutcome::Captured)
            }
            Event::Mouse(mouse::Event::CursorMoved { position })
                if self.source == Some(TabDragSource::Mouse) =>
            {
                self.move_to(*position);
                Some(TabDragOutcome::Captured)
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if self.source == Some(TabDragSource::Mouse) =>
            {
                Some(self.finish(bounds, disabled))
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) if self.source.is_none() => {
                let index = tab_at(bounds, disabled, *position)?;
                self.begin(TabDragSource::Touch(*id), index, *position);
                Some(TabDragOutcome::Captured)
            }
            Event::Touch(touch::Event::FingerMoved { id, position })
                if self.source == Some(TabDragSource::Touch(*id)) =>
            {
                self.move_to(*position);
                Some(TabDragOutcome::Captured)
            }
            Event::Touch(touch::Event::FingerLifted { id, position })
                if self.source == Some(TabDragSource::Touch(*id)) =>
            {
                self.move_to(*position);
                Some(self.finish(bounds, disabled))
            }
            Event::Touch(touch::Event::FingerLost { id, .. })
                if self.source == Some(TabDragSource::Touch(*id)) =>
            {
                self.clear();
                Some(TabDragOutcome::Cancelled)
            }
            Event::Window(iced::window::Event::Unfocused) if self.source.is_some() => {
                self.clear();
                Some(TabDragOutcome::Cancelled)
            }
            _ => None,
        }
    }

    fn begin(&mut self, source: TabDragSource, index: usize, position: Point) {
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
            self.moved |=
                delta.x * delta.x + delta.y * delta.y >= TAB_DRAG_THRESHOLD * TAB_DRAG_THRESHOLD;
        }
    }

    fn finish(&mut self, bounds: &[Rectangle], disabled: &[bool]) -> TabDragOutcome {
        let source = self.active_index.expect("active tab drag has a source");
        let position = self.position.expect("active tab drag has a position");
        let outcome = if self.moved {
            TabDragOutcome::Reorder {
                source,
                before: drop_before_index(bounds, disabled, Some(source), self.position),
                position,
            }
        } else {
            TabDragOutcome::Select(source)
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

fn tab_at(bounds: &[Rectangle], disabled: &[bool], position: Point) -> Option<usize> {
    bounds
        .iter()
        .enumerate()
        .find(|(index, bounds)| {
            !disabled.get(*index).copied().unwrap_or(true) && bounds.contains(position)
        })
        .map(|(index, _)| index)
}

fn drop_before_index(
    bounds: &[Rectangle],
    disabled: &[bool],
    excluded: Option<usize>,
    position: Option<Point>,
) -> Option<usize> {
    let position = position?;
    bounds
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            Some(*index) != excluded && !disabled.get(*index).copied().unwrap_or(false)
        })
        .find(|(_, bounds)| position.x < bounds.center_x())
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

pub(crate) struct DraggableTabStrip<'a, T, Message> {
    content: Element<'a, Message>,
    values: Vec<T>,
    disabled: Vec<bool>,
    on_select: Rc<dyn Fn(T) -> Message + 'a>,
    on_reorder: Option<TabReorderHandler<'a, T, Message>>,
    transfer: Option<TabTransfer<'a, T, Message>>,
    indicator: Color,
}

type TabTransferHandler<'a, T, Message> = Rc<dyn Fn(String, T, String, Option<T>) -> Message + 'a>;
type TabReorderHandler<'a, T, Message> = Rc<dyn Fn(T, Option<T>) -> Message + 'a>;

pub(crate) struct TabDragSetup<'a, T, Message> {
    group: TabDragGroup<T>,
    surface: TabDragSurface,
    strip_id: String,
    on_transfer: TabTransferHandler<'a, T, Message>,
    accepts_external_drop: bool,
}

impl<'a, T, Message> TabDragSetup<'a, T, Message> {
    pub(crate) fn new(
        group: TabDragGroup<T>,
        surface: TabDragSurface,
        strip_id: String,
        on_transfer: TabTransferHandler<'a, T, Message>,
        accepts_external_drop: bool,
    ) -> Self {
        Self {
            group,
            surface,
            strip_id,
            on_transfer,
            accepts_external_drop,
        }
    }
}

struct TabTransfer<'a, T, Message> {
    lease: TabDragLease<T>,
    on_transfer: TabTransferHandler<'a, T, Message>,
    accepts_external_drop: bool,
}

impl<'a, T, Message> DraggableTabStrip<'a, T, Message> {
    pub(crate) fn new(
        content: impl Into<Element<'a, Message>>,
        values: Vec<T>,
        disabled: Vec<bool>,
        on_select: Rc<dyn Fn(T) -> Message + 'a>,
        on_reorder: Option<TabReorderHandler<'a, T, Message>>,
        drag_group: Option<TabDragSetup<'a, T, Message>>,
        indicator: Color,
    ) -> Self {
        let transfer = drag_group.map(|setup| TabTransfer {
            lease: setup.group.lease(setup.surface, setup.strip_id),
            on_transfer: setup.on_transfer,
            accepts_external_drop: setup.accepts_external_drop,
        });
        Self {
            content: content.into(),
            values,
            disabled,
            on_select,
            on_reorder,
            transfer,
            indicator,
        }
    }
}

impl<T, Message> Widget<Message, Theme, iced::Renderer> for DraggableTabStrip<'_, T, Message>
where
    T: Clone,
    Message: Clone,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<TabDragState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(TabDragState::default())
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(&mut [self.content.as_widget_mut()]);
        let state = tree.state.downcast_mut::<TabDragState>();
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
        let content_layout = layout.children().next().expect("tab strip content");
        let bounds = content_layout
            .children()
            .map(|tab| tab.bounds())
            .collect::<Vec<_>>();
        let state = tree.state.downcast_mut::<TabDragState>();
        if state.source.is_none()
            && let Some(transfer) = self.transfer.as_ref()
        {
            let relayed_position = match event {
                Event::Mouse(mouse::Event::CursorMoved { position })
                | Event::Touch(touch::Event::FingerMoved { position, .. }) => Some(*position),
                _ => None,
            };
            if relayed_position.is_some_and(|position| {
                transfer
                    .lease
                    .group
                    .relay_pointer(&transfer.lease.surface, drag_point(position))
            }) {
                shell.capture_event();
                shell.request_redraw();
                return;
            }
            let release_position = match event {
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    cursor.position()
                }
                Event::Touch(touch::Event::FingerLifted { position, .. }) => Some(*position),
                _ => None,
            };
            if let Some((source_strip, value, target_strip, before)) =
                release_position.and_then(|position| {
                    transfer.lease.group.finish_relay(
                        &transfer.lease.surface,
                        &transfer.lease.strip_id,
                        drag_point(position),
                    )
                })
            {
                shell.publish((transfer.on_transfer)(
                    source_strip,
                    value,
                    target_strip,
                    before,
                ));
                shell.capture_event();
                shell.request_redraw();
                return;
            }
        }
        let outcome = state.update(event, &bounds, &self.disabled, cursor);
        if let (Some(transfer), Some(source), Some(position)) =
            (self.transfer.as_ref(), state.active_index, state.position)
        {
            transfer.lease.group.sync_active(
                &transfer.lease.surface,
                &transfer.lease.strip_id,
                transfer.lease.generation,
                source,
                drag_point(position),
                state.moved,
            );
        }
        if let Some(outcome) = outcome {
            match outcome {
                TabDragOutcome::Select(index) => {
                    if let Some(transfer) = self.transfer.as_ref() {
                        transfer
                            .lease
                            .group
                            .clear_active(&transfer.lease.strip_id, transfer.lease.generation);
                    }
                    if let Some(value) = self.values.get(index).cloned() {
                        shell.publish((self.on_select)(value));
                    }
                }
                TabDragOutcome::Reorder {
                    source,
                    before,
                    position,
                } => {
                    let relayed_drop_completed = self.transfer.as_ref().is_some_and(|transfer| {
                        transfer
                            .lease
                            .group
                            .take_completed(&transfer.lease.strip_id, transfer.lease.generation)
                    });
                    if !relayed_drop_completed && let Some(value) = self.values.get(source).cloned()
                    {
                        let cross_drop = self.transfer.as_ref().and_then(|transfer| {
                            transfer
                                .lease
                                .group
                                .cross_drop(
                                    &transfer.lease.surface,
                                    &transfer.lease.strip_id,
                                    drag_point(position),
                                )
                                .map(|(target_strip, before)| {
                                    (
                                        transfer.lease.strip_id.clone(),
                                        target_strip,
                                        before,
                                        Rc::clone(&transfer.on_transfer),
                                    )
                                })
                        });
                        if let Some((source_strip, target_strip, before, on_transfer)) = cross_drop
                        {
                            shell.publish(on_transfer(source_strip, value, target_strip, before));
                        } else if reorder_changes_position(self.values.len(), source, before)
                            && let Some(on_reorder) = self.on_reorder.as_ref()
                        {
                            let before = before.and_then(|index| self.values.get(index).cloned());
                            shell.publish(on_reorder(value, before));
                        }
                    }
                    if let Some(transfer) = self.transfer.as_ref() {
                        transfer
                            .lease
                            .group
                            .clear_active(&transfer.lease.strip_id, transfer.lease.generation);
                    }
                }
                TabDragOutcome::Cancelled => {
                    if let Some(transfer) = self.transfer.as_ref() {
                        transfer
                            .lease
                            .group
                            .clear_active(&transfer.lease.strip_id, transfer.lease.generation);
                    }
                }
                TabDragOutcome::Captured => {}
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
        let content_layout = layout.children().next().expect("tab strip content");
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
            .map(|tab| tab.bounds())
            .collect::<Vec<_>>();
        let indicator = if let Some(transfer) = self.transfer.as_ref() {
            transfer.lease.group.register(
                &transfer.lease.surface,
                &transfer.lease.strip_id,
                transfer.lease.generation,
                strip_paint(
                    content_layout.bounds(),
                    &bounds,
                    &self.values,
                    &self.disabled,
                    transfer.accepts_external_drop,
                ),
            );
            transfer.lease.group.indicator_for(&transfer.lease.strip_id)
        } else {
            let state = tree.state.downcast_ref::<TabDragState>();
            state
                .moved
                .then_some(state.active_index)
                .flatten()
                .map(|source| TabDropIndicator {
                    before: drop_before_index(
                        &bounds,
                        &self.disabled,
                        Some(source),
                        state.position,
                    ),
                    source: Some(source),
                })
        };
        let Some(TabDropIndicator { before, source }) = indicator else {
            return;
        };
        if source.is_some_and(|source| !reorder_changes_position(bounds.len(), source, before)) {
            return;
        }
        let x = before
            .and_then(|index| bounds.get(index).map(|bounds| bounds.x - 2.0))
            .or_else(|| bounds.last().map(|bounds| bounds.x + bounds.width + 2.0));
        let Some(x) = x else {
            return;
        };
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle::new(
                    Point::new(x, content_layout.bounds().y + 4.0),
                    Size::new(2.0, (content_layout.bounds().height - 8.0).max(0.0)),
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
        let content_layout = layout.children().next().expect("tab strip content");
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
        let content_layout = layout.children().next().expect("tab strip content");
        let state = tree.state.downcast_ref::<TabDragState>();
        let shared_drag_is_over = self.transfer.as_ref().is_some_and(|transfer| {
            cursor.position().is_some_and(|position| {
                transfer.lease.group.is_active_over(
                    &transfer.lease.surface,
                    &transfer.lease.strip_id,
                    drag_point(position),
                )
            })
        });
        if state.source.is_some() || shared_drag_is_over {
            mouse::Interaction::Grabbing
        } else {
            let bounds = content_layout
                .children()
                .map(|tab| tab.bounds())
                .collect::<Vec<_>>();
            if cursor
                .position()
                .and_then(|position| tab_at(&bounds, &self.disabled, position))
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
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        let content_layout = layout.children().next().expect("tab strip content");
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            content_layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, T, Message> From<DraggableTabStrip<'a, T, Message>> for Element<'a, Message>
where
    T: Clone + 'a,
    Message: Clone + 'a,
{
    fn from(strip: DraggableTabStrip<'a, T, Message>) -> Self {
        Element::new(strip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Vec<Rectangle> {
        (0..3)
            .map(|index| {
                Rectangle::new(Point::new(index as f32 * 80.0, 0.0), Size::new(76.0, 28.0))
            })
            .collect()
    }

    #[test]
    fn click_selects_while_drag_reorders_by_tab_midpoint() {
        let bounds = bounds();
        let disabled = [false; 3];
        let mut state = TabDragState::default();
        let start = Point::new(118.0, 14.0);

        assert_eq!(
            state.update(
                &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                &bounds,
                &disabled,
                mouse::Cursor::Available(start),
            ),
            Some(TabDragOutcome::Captured)
        );
        assert_eq!(
            state.update(
                &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                &bounds,
                &disabled,
                mouse::Cursor::Available(start),
            ),
            Some(TabDragOutcome::Select(1))
        );

        state.update(
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            &bounds,
            &disabled,
            mouse::Cursor::Available(start),
        );
        state.update(
            &Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(10.0, 14.0),
            }),
            &bounds,
            &disabled,
            mouse::Cursor::Available(Point::new(10.0, 14.0)),
        );
        assert_eq!(
            state.update(
                &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                &bounds,
                &disabled,
                mouse::Cursor::Available(Point::new(10.0, 14.0)),
            ),
            Some(TabDragOutcome::Reorder {
                source: 1,
                before: Some(0),
                position: Point::new(10.0, 14.0),
            })
        );
    }

    #[test]
    fn disabled_tabs_do_not_start_drag_and_unfocus_cancels_active_drag() {
        let bounds = bounds();
        let mut state = TabDragState::default();
        assert_eq!(
            state.update(
                &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                &bounds,
                &[false, true, false],
                mouse::Cursor::Available(Point::new(118.0, 14.0)),
            ),
            None
        );

        state.update(
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            &bounds,
            &[false; 3],
            mouse::Cursor::Available(Point::new(20.0, 14.0)),
        );
        assert_eq!(
            state.update(
                &Event::Window(iced::window::Event::Unfocused),
                &bounds,
                &[false; 3],
                mouse::Cursor::Unavailable,
            ),
            Some(TabDragOutcome::Cancelled)
        );
        assert!(state.source.is_none());
    }

    #[test]
    fn reorder_contract_filters_noop_insertions() {
        assert!(!reorder_changes_position(3, 0, Some(1)));
        assert!(!reorder_changes_position(3, 1, Some(2)));
        assert!(!reorder_changes_position(3, 2, None));
        assert!(reorder_changes_position(3, 2, Some(0)));
        assert!(reorder_changes_position(3, 0, None));
    }

    #[test]
    fn disabled_tabs_are_skipped_as_drop_targets() {
        assert_eq!(
            drop_before_index(
                &bounds(),
                &[true, false, false],
                Some(2),
                Some(Point::new(1.0, 14.0)),
            ),
            Some(1)
        );
    }
}
