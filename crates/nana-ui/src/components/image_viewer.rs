use std::borrow::Cow;

use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, renderer};
use iced::widget::{button, column, container, mouse_area, row, stack, text};
use iced::{
    Element, Event, Length, Padding, Point, Rectangle, Size, Theme, Transformation, Vector, touch,
};

use crate::icons::{Icon, icon};
use crate::theme::{ThemeTokens, UI_METRICS, ui_font};
use crate::widgets::{ButtonKind, button_style};

/// Host-rendered visual content and the human-readable metadata shown by
/// [`ImageViewer`].
///
/// The content can be an application-decoded Iced image, a NanaUI
/// [`crate::GpuTextureView`], or any other renderer-native element. NanaUI does
/// not force image codecs or application-side pixel copies into every binary.
pub struct ImageViewerSource<'a, Message> {
    pub content: Element<'a, Message>,
    pub name: Option<Cow<'a, str>>,
    pub metadata: Option<Cow<'a, str>>,
}

impl<'a, Message> ImageViewerSource<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
            name: None,
            metadata: None,
        }
    }

    pub fn name(mut self, name: impl Into<Cow<'a, str>>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn metadata(mut self, metadata: impl Into<Cow<'a, str>>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }
}

/// A full-window viewer that zooms and pans renderer-native content.
///
/// Escape remains a host subscription, like [`super::Dialog`], while close
/// and outside presses use distinct messages.
pub struct ImageViewer<'a, Message> {
    source: ImageViewerSource<'a, Message>,
    on_close: Message,
    on_outside: Message,
    on_interaction: Message,
    tokens: ThemeTokens,
}

impl<'a, Message> ImageViewer<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        source: ImageViewerSource<'a, Message>,
        on_close: Message,
        on_outside: Message,
        on_interaction: Message,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            source,
            on_close,
            on_outside,
            on_interaction,
            tokens: theme.into(),
        }
    }

    pub fn view(self) -> Element<'a, Message> {
        let colors = self.tokens.colors;
        let viewer = Element::new(ZoomPan::new(self.source.content));
        let stage = container(viewer)
            .width(Length::Fill)
            .height(Length::Fill)
            .clip(true)
            .style(move |_theme| {
                iced::widget::container::Style::default()
                    .background(colors.background.scale_alpha(0.34))
            });

        let mut figure = column![stage]
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fill);
        if self.source.name.is_some() || self.source.metadata.is_some() {
            let mut metadata = row![].spacing(10).align_y(iced::Alignment::Center);
            if let Some(name) = self.source.name {
                metadata = metadata.push(
                    text(name)
                        .size(12)
                        .font(ui_font(iced::font::Weight::Semibold))
                        .color(colors.text),
                );
            }
            if let Some(details) = self.source.metadata {
                metadata = metadata.push(text(details).size(11).color(colors.muted));
            }
            figure = figure.push(
                container(metadata)
                    .width(Length::Fill)
                    .center_x(Length::Fill),
            );
        }

        let close = button(icon(Icon::Close, 16.0, colors.muted))
            .width(Length::Fixed(UI_METRICS.icon_button_size))
            .height(Length::Fixed(UI_METRICS.icon_button_size))
            .padding(0)
            .on_press(self.on_close)
            .style(button_style(self.tokens, ButtonKind::Subtle));
        let close = container(close)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_right(Length::Fill)
            .align_top(Length::Fill)
            .padding(14);
        let surface = container(stack![figure, close])
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: 54.0,
                right: 54.0,
                bottom: 24.0,
                left: 54.0,
            });
        let surface = mouse_area(surface)
            .on_press(self.on_interaction)
            .interaction(iced::mouse::Interaction::Idle);
        let overlay = container(surface)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| {
                iced::widget::container::Style::default()
                    .background(colors.background.scale_alpha(0.94))
                    .color(colors.text)
            });
        mouse_area(overlay).on_press(self.on_outside).into()
    }
}

struct ZoomPan<'a, Message> {
    content: Element<'a, Message>,
}

impl<'a, Message> ZoomPan<'a, Message> {
    fn new(content: Element<'a, Message>) -> Self {
        Self { content }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragSource {
    Mouse,
    Touch(touch::Finger),
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    source: DragSource,
    origin: Point,
    starting_offset: Vector,
}

#[derive(Debug)]
struct ZoomPanState {
    scale: f32,
    offset: Vector,
    drag: Option<Drag>,
}

impl Default for ZoomPanState {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset: Vector::ZERO,
            drag: None,
        }
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for ZoomPan<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<ZoomPanState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(ZoomPanState::default())
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(&mut [self.content.as_widget_mut()]);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.max();
        let child = self.content.as_widget_mut().layout(
            &mut tree.children[0],
            renderer,
            &layout::Limits::new(Size::ZERO, size)
                .width(Length::Fill)
                .height(Length::Fill),
        );
        layout::Node::with_children(size, vec![child])
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
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<ZoomPanState>();
        match event {
            Event::Mouse(iced::mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let y = match delta {
                    iced::mouse::ScrollDelta::Lines { y, .. }
                    | iced::mouse::ScrollDelta::Pixels { y, .. } => *y,
                };
                let previous = state.scale;
                state.scale = if y > 0.0 {
                    state.scale * 1.12
                } else {
                    state.scale / 1.12
                }
                .clamp(1.0, 6.0);
                let factor = state.scale / previous - 1.0;
                if let Some(position) = cursor.position_over(bounds) {
                    let cursor_to_center = position - bounds.center();
                    state.offset = clamp_offset(
                        state.offset + cursor_to_center * factor + state.offset * factor,
                        state.scale,
                        bounds.size(),
                    );
                }
                shell.request_redraw();
                shell.capture_event();
                return;
            }
            Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
                if state.scale > 1.0 =>
            {
                if let Some(position) = cursor.position_over(bounds) {
                    state.drag = Some(Drag {
                        source: DragSource::Mouse,
                        origin: position,
                        starting_offset: state.offset,
                    });
                    shell.capture_event();
                    return;
                }
            }
            Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                if let Some(drag) = state.drag
                    && drag.source == DragSource::Mouse
                {
                    state.offset = clamp_offset(
                        drag.starting_offset + (*position - drag.origin),
                        state.scale,
                        bounds.size(),
                    );
                    shell.request_redraw();
                    shell.capture_event();
                    return;
                }
            }
            Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))
                if state
                    .drag
                    .is_some_and(|drag| drag.source == DragSource::Mouse) =>
            {
                state.drag = None;
                shell.capture_event();
                return;
            }
            Event::Touch(touch::Event::FingerPressed { id, position })
                if state.scale > 1.0 && bounds.contains(*position) =>
            {
                state.drag = Some(Drag {
                    source: DragSource::Touch(*id),
                    origin: *position,
                    starting_offset: state.offset,
                });
                shell.capture_event();
                return;
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) => {
                if let Some(drag) = state.drag
                    && drag.source == DragSource::Touch(*id)
                {
                    state.offset = clamp_offset(
                        drag.starting_offset + (*position - drag.origin),
                        state.scale,
                        bounds.size(),
                    );
                    shell.request_redraw();
                    shell.capture_event();
                    return;
                }
            }
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) if state
                .drag
                .is_some_and(|drag| drag.source == DragSource::Touch(*id)) =>
            {
                state.drag = None;
                shell.capture_event();
                return;
            }
            _ => {}
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().expect("zoom content layout"),
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
        use iced::advanced::Renderer as _;

        let bounds = layout.bounds();
        let state = tree.state.downcast_ref::<ZoomPanState>();
        let center = bounds.center();
        let transformation =
            Transformation::translate(center.x + state.offset.x, center.y + state.offset.y)
                * Transformation::scale(state.scale)
                * Transformation::translate(-center.x, -center.y);
        renderer.with_layer(bounds, |renderer| {
            renderer.with_transformation(transformation, |renderer| {
                self.content.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    style,
                    layout.children().next().expect("zoom content layout"),
                    cursor,
                    viewport,
                );
            });
        });
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().expect("zoom content layout"),
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
        let state = tree.state.downcast_ref::<ZoomPanState>();
        if cursor.is_over(layout.bounds()) && state.drag.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(layout.bounds()) && state.scale > 1.0 {
            mouse::Interaction::Grab
        } else {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout.children().next().expect("zoom content layout"),
                cursor,
                viewport,
                renderer,
            )
        }
    }
}

fn clamp_offset(offset: Vector, scale: f32, viewport: Size) -> Vector {
    if scale <= 1.0 {
        return Vector::ZERO;
    }
    Vector::new(
        clamp_axis(offset.x, viewport.width * scale, viewport.width),
        clamp_axis(offset.y, viewport.height * scale, viewport.height),
    )
}

fn clamp_axis(value: f32, rendered: f32, viewport: f32) -> f32 {
    let required_coverage = viewport * 0.75;
    let max = ((viewport + rendered) / 2.0 - required_coverage).max(0.0);
    value.clamp(-max, max)
}

#[cfg(test)]
mod tests {
    use super::clamp_offset;
    use iced::{Size, Vector};

    #[test]
    fn pan_clamp_keeps_required_content_coverage_visible() {
        assert_eq!(
            clamp_offset(Vector::new(500.0, -500.0), 2.0, Size::new(100.0, 80.0)),
            Vector::new(75.0, -60.0)
        );
        assert_eq!(
            clamp_offset(Vector::new(20.0, 20.0), 1.0, Size::new(100.0, 80.0)),
            Vector::ZERO
        );
    }
}
