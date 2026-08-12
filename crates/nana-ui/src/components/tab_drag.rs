use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use iced::advanced::Renderer as _;
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Theme, Vector, touch};

const TAB_DRAG_THRESHOLD: f32 = 4.0;

/// Shared geometry and active-drag state for tabs that can move between strips.
///
/// Each [`Tabs`](super::controls::Tabs) instance registers its current painted
/// bounds through a short-lived lease. The group owns no application ordering;
/// it only resolves a pointer release to a target strip and a before-value.
pub struct TabDragGroup<T> {
    inner: Rc<RefCell<TabDragGroupState<T>>>,
}

/// Window-local coordinate transform used by a [`TabDragGroup`].
#[derive(Debug, Clone, PartialEq)]
pub struct TabDragSurface {
    id: String,
    physical_origin: Point,
    scale_factor: f32,
}

impl TabDragSurface {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            physical_origin: Point::ORIGIN,
            scale_factor: 1.0,
        }
    }

    /// Sets the window origin in physical screen pixels and its logical scale.
    pub fn with_physical_geometry(mut self, x: i32, y: i32, scale_factor: f64) -> Self {
        self.physical_origin = Point::new(x as f32, y as f32);
        self.scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor as f32
        } else {
            1.0
        };
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn global_point(&self, point: Point) -> Point {
        Point::new(
            self.physical_origin.x + point.x * self.scale_factor,
            self.physical_origin.y + point.y * self.scale_factor,
        )
    }

    fn global_rectangle(&self, rectangle: Rectangle) -> Rectangle {
        Rectangle::new(
            self.global_point(rectangle.position()),
            Size::new(
                rectangle.width * self.scale_factor,
                rectangle.height * self.scale_factor,
            ),
        )
    }
}

impl<T> Clone for TabDragGroup<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T> Default for TabDragGroup<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TabDragGroup<T> {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(TabDragGroupState::default())),
        }
    }

    fn lease(&self, surface: TabDragSurface, strip_id: String) -> TabDragLease<T> {
        let mut state = self.inner.borrow_mut();
        state.next_generation = state.next_generation.saturating_add(1);
        let generation = state.next_generation;
        TabDragLease {
            group: self.clone(),
            surface,
            strip_id,
            generation,
        }
    }

    fn register(
        &self,
        surface: &TabDragSurface,
        strip_id: &str,
        generation: u64,
        paint: TabStripPaint<T>,
    ) {
        self.inner.borrow_mut().strips.insert(
            strip_id.to_owned(),
            TabStripRegistration {
                generation,
                surface_id: surface.id.clone(),
                bounds: surface.global_rectangle(paint.bounds),
                tab_bounds: paint
                    .tab_bounds
                    .into_iter()
                    .map(|bounds| surface.global_rectangle(bounds))
                    .collect(),
                values: paint.values,
                disabled: paint.disabled,
                accepts_external_drop: paint.accepts_external_drop,
            },
        );
    }

    fn sync_active(
        &self,
        surface: &TabDragSurface,
        source_strip: &str,
        source_generation: u64,
        source_index: usize,
        position: Point,
        moved: bool,
    ) {
        let mut state = self.inner.borrow_mut();
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.source_strip != source_strip)
        {
            return;
        }
        state.active = Some(ActiveGroupDrag {
            source_surface: surface.id.clone(),
            source_strip: source_strip.to_owned(),
            source_generation,
            source_index,
            position: surface.global_point(position),
            moved,
        });
        state.completed_source = None;
    }

    fn clear_active(&self, source_strip: &str, source_generation: u64) {
        let mut state = self.inner.borrow_mut();
        if state.active.as_ref().is_some_and(|active| {
            active.source_strip == source_strip && active.source_generation == source_generation
        }) {
            state.active = None;
        }
    }

    fn relay_pointer(&self, surface: &TabDragSurface, position: Point) -> bool {
        let mut state = self.inner.borrow_mut();
        let Some(active) = state.active.as_mut() else {
            return false;
        };
        if active.source_surface == surface.id {
            return false;
        }
        active.position = surface.global_point(position);
        active.moved = true;
        true
    }

    fn take_completed(&self, source_strip: &str, source_generation: u64) -> bool {
        let mut state = self.inner.borrow_mut();
        if state.completed_source.as_ref().is_some_and(|completed| {
            completed.0 == source_strip && completed.1 == source_generation
        }) {
            state.completed_source = None;
            true
        } else {
            false
        }
    }
}

impl<T: Clone> TabDragGroup<T> {
    fn cross_drop(
        &self,
        surface: &TabDragSurface,
        source_strip: &str,
        position: Point,
    ) -> Option<(String, Option<T>)> {
        let position = surface.global_point(position);
        let state = self.inner.borrow();
        let (target_id, target) = state.strips.iter().find(|(strip_id, strip)| {
            strip_id.as_str() != source_strip
                && strip.accepts_external_drop
                && strip.bounds.contains(position)
        })?;
        let before = drop_before_index(&target.tab_bounds, &target.disabled, None, Some(position))
            .and_then(|index| target.values.get(index).cloned());
        Some((target_id.clone(), before))
    }

    fn indicator_for(&self, strip_id: &str) -> Option<TabDropIndicator> {
        let state = self.inner.borrow();
        let active = state.active.as_ref().filter(|active| active.moved)?;
        let strip = state.strips.get(strip_id)?;
        if active.source_strip != strip_id && !strip.accepts_external_drop {
            return None;
        }
        if !strip.bounds.contains(active.position) {
            return None;
        }
        let excluded = (active.source_strip == strip_id).then_some(active.source_index);
        Some(TabDropIndicator {
            before: drop_before_index(
                &strip.tab_bounds,
                &strip.disabled,
                excluded,
                Some(active.position),
            ),
            source: excluded,
        })
    }

    fn is_active_over(&self, surface: &TabDragSurface, strip_id: &str, position: Point) -> bool {
        let position = surface.global_point(position);
        let state = self.inner.borrow();
        state.active.as_ref().is_some_and(|active| {
            active.moved
                && state.strips.get(strip_id).is_some_and(|strip| {
                    (active.source_strip == strip_id || strip.accepts_external_drop)
                        && strip.bounds.contains(position)
                })
        })
    }

    fn finish_relay(
        &self,
        surface: &TabDragSurface,
        target_strip: &str,
        position: Point,
    ) -> Option<(String, T, String, Option<T>)> {
        let position = surface.global_point(position);
        let mut state = self.inner.borrow_mut();
        let active = state.active.clone()?;
        if active.source_surface == surface.id {
            return None;
        }
        let source = state.strips.get(&active.source_strip)?;
        if source.generation != active.source_generation {
            return None;
        }
        let value = source.values.get(active.source_index)?.clone();
        let target = state.strips.get(target_strip)?;
        if target.surface_id != surface.id
            || !target.accepts_external_drop
            || !target.bounds.contains(position)
        {
            return None;
        }
        let before = drop_before_index(&target.tab_bounds, &target.disabled, None, Some(position))
            .and_then(|index| target.values.get(index).cloned());
        state.active = None;
        state.completed_source = Some((active.source_strip.clone(), active.source_generation));
        Some((active.source_strip, value, target_strip.to_owned(), before))
    }
}

struct TabDragGroupState<T> {
    next_generation: u64,
    strips: BTreeMap<String, TabStripRegistration<T>>,
    active: Option<ActiveGroupDrag>,
    completed_source: Option<(String, u64)>,
}

impl<T> Default for TabDragGroupState<T> {
    fn default() -> Self {
        Self {
            next_generation: 0,
            strips: BTreeMap::new(),
            active: None,
            completed_source: None,
        }
    }
}

struct TabStripRegistration<T> {
    generation: u64,
    surface_id: String,
    bounds: Rectangle,
    tab_bounds: Vec<Rectangle>,
    values: Vec<T>,
    disabled: Vec<bool>,
    accepts_external_drop: bool,
}

struct TabStripPaint<T> {
    bounds: Rectangle,
    tab_bounds: Vec<Rectangle>,
    values: Vec<T>,
    disabled: Vec<bool>,
    accepts_external_drop: bool,
}

#[derive(Debug, Clone)]
struct ActiveGroupDrag {
    source_surface: String,
    source_strip: String,
    source_generation: u64,
    source_index: usize,
    position: Point,
    moved: bool,
}

#[derive(Debug, Clone, Copy)]
struct TabDropIndicator {
    before: Option<usize>,
    source: Option<usize>,
}

struct TabDragLease<T> {
    group: TabDragGroup<T>,
    surface: TabDragSurface,
    strip_id: String,
    generation: u64,
}

impl<T> Drop for TabDragLease<T> {
    fn drop(&mut self) {
        let mut state = self.group.inner.borrow_mut();
        if state
            .strips
            .get(&self.strip_id)
            .is_some_and(|strip| strip.generation == self.generation)
        {
            state.strips.remove(&self.strip_id);
        }
        if state.active.as_ref().is_some_and(|active| {
            active.source_strip == self.strip_id && active.source_generation == self.generation
        }) {
            state.active = None;
        }
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
                    .relay_pointer(&transfer.lease.surface, position)
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
                        position,
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
                position,
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
                                    position,
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
                TabStripPaint {
                    bounds: content_layout.bounds(),
                    tab_bounds: bounds.clone(),
                    values: self.values.clone(),
                    disabled: self.disabled.clone(),
                    accepts_external_drop: transfer.accepts_external_drop,
                },
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
                    position,
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

    #[test]
    fn drag_group_resolves_another_strip_and_its_before_value() {
        let group = TabDragGroup::new();
        let surface = TabDragSurface::new("default");
        let source = group.lease(surface.clone(), "left".to_owned());
        let target = group.lease(surface.clone(), "right".to_owned());
        let source_bounds = bounds();
        let target_bounds = bounds()
            .into_iter()
            .map(|bounds| Rectangle::new(Point::new(bounds.x + 300.0, 0.0), bounds.size()))
            .collect::<Vec<_>>();
        group.register(
            &surface,
            &source.strip_id,
            source.generation,
            TabStripPaint {
                bounds: Rectangle::new(Point::ORIGIN, Size::new(236.0, 28.0)),
                tab_bounds: source_bounds,
                values: vec!["overview", "a", "b"],
                disabled: vec![true, false, false],
                accepts_external_drop: true,
            },
        );
        group.register(
            &surface,
            &target.strip_id,
            target.generation,
            TabStripPaint {
                bounds: Rectangle::new(Point::new(300.0, 0.0), Size::new(236.0, 28.0)),
                tab_bounds: target_bounds,
                values: vec!["overview", "c", "d"],
                disabled: vec![true, false, false],
                accepts_external_drop: true,
            },
        );
        let position = Point::new(301.0, 14.0);

        assert_eq!(
            group.cross_drop(&surface, "left", position),
            Some(("right".to_owned(), Some("c")))
        );
        group.sync_active(&surface, "left", source.generation, 2, position, true);
        assert_eq!(
            group
                .indicator_for("right")
                .map(|indicator| indicator.before),
            Some(Some(1))
        );
        assert!(group.is_active_over(&surface, "right", position));
    }

    #[test]
    fn newer_strip_lease_survives_an_older_view_drop() {
        let group = TabDragGroup::<u8>::new();
        let surface = TabDragSurface::new("default");
        let older = group.lease(surface.clone(), "pane".to_owned());
        group.register(
            &surface,
            &older.strip_id,
            older.generation,
            TabStripPaint {
                bounds: Rectangle::new(Point::ORIGIN, Size::new(80.0, 28.0)),
                tab_bounds: vec![bounds()[0]],
                values: vec![1],
                disabled: vec![false],
                accepts_external_drop: true,
            },
        );
        let newer = group.lease(surface.clone(), "pane".to_owned());
        group.register(
            &surface,
            &newer.strip_id,
            newer.generation,
            TabStripPaint {
                bounds: Rectangle::new(Point::ORIGIN, Size::new(80.0, 28.0)),
                tab_bounds: vec![bounds()[0]],
                values: vec![2],
                disabled: vec![false],
                accepts_external_drop: true,
            },
        );

        drop(older);
        assert_eq!(
            group.inner.borrow().strips["pane"].generation,
            newer.generation
        );
    }

    #[test]
    fn drag_group_relays_between_scaled_window_surfaces_once() {
        let group = TabDragGroup::new();
        let source_surface =
            TabDragSurface::new("source-window").with_physical_geometry(100, 100, 2.0);
        let target_surface =
            TabDragSurface::new("target-window").with_physical_geometry(500, 120, 1.5);
        let source = group.lease(source_surface.clone(), "source-pane".to_owned());
        let target = group.lease(target_surface.clone(), "target-pane".to_owned());
        group.register(
            &source_surface,
            &source.strip_id,
            source.generation,
            TabStripPaint {
                bounds: Rectangle::new(Point::ORIGIN, Size::new(236.0, 28.0)),
                tab_bounds: bounds(),
                values: vec![0, 1, 2],
                disabled: vec![true, false, false],
                accepts_external_drop: false,
            },
        );
        group.register(
            &target_surface,
            &target.strip_id,
            target.generation,
            TabStripPaint {
                bounds: Rectangle::new(Point::ORIGIN, Size::new(236.0, 28.0)),
                tab_bounds: bounds(),
                values: vec![0, 3, 4],
                disabled: vec![true, false, false],
                accepts_external_drop: true,
            },
        );
        group.sync_active(
            &source_surface,
            &source.strip_id,
            source.generation,
            2,
            Point::new(198.0, 14.0),
            true,
        );

        assert!(group.relay_pointer(&target_surface, Point::new(1.0, 14.0)));
        assert_eq!(
            group.finish_relay(&target_surface, &target.strip_id, Point::new(1.0, 14.0)),
            Some((
                "source-pane".to_owned(),
                2,
                "target-pane".to_owned(),
                Some(3),
            ))
        );
        assert!(group.take_completed(&source.strip_id, source.generation));
        assert!(!group.take_completed(&source.strip_id, source.generation));
    }
}
