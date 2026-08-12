use std::borrow::Cow;

use iced::advanced::Renderer as _;
use iced::advanced::text::{Paragraph, Renderer as TextRenderer};
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Layout, Shell, Text, clipboard, layout, mouse, renderer};
use iced::keyboard;
use iced::widget::text::{self, Alignment, Ellipsis, LineHeight, Shaping, Span, Style, Wrapping};
use iced::{Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size, Theme, Vector};
use unicode_segmentation::UnicodeSegmentation;

type RendererParagraph = <iced::Renderer as TextRenderer>::Paragraph;

pub struct SelectableRichText<Message> {
    spans: Vec<Span<'static, String, Font>>,
    size: Option<Pixels>,
    line_height: LineHeight,
    width: Length,
    align_x: Alignment,
    color: Option<Color>,
    selection_color: Color,
    on_link_click: Option<Box<dyn Fn(String) -> Message>>,
}

impl<Message> SelectableRichText<Message> {
    pub fn new(spans: Vec<Span<'static, String, Font>>) -> Self {
        Self {
            spans,
            size: None,
            line_height: LineHeight::default(),
            width: Length::Shrink,
            align_x: Alignment::Default,
            color: None,
            selection_color: Color::from_rgba(0.25, 0.48, 0.9, 0.3),
            on_link_click: None,
        }
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn align_x(mut self, alignment: impl Into<Alignment>) -> Self {
        self.align_x = alignment.into();
        self
    }

    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = Some(color.into());
        self
    }

    pub fn selection_color(mut self, color: impl Into<Color>) -> Self {
        self.selection_color = color.into();
        self
    }

    pub fn on_link_click(mut self, on_link_click: impl Fn(String) -> Message + 'static) -> Self {
        self.on_link_click = Some(Box::new(on_link_click));
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    anchor: usize,
    focus: usize,
}

impl Selection {
    fn ordered(self) -> (usize, usize) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    fn is_empty(self) -> bool {
        self.anchor == self.focus
    }
}

#[derive(Debug, Default)]
struct SelectableRichTextState {
    spans: Vec<Span<'static, String, Font>>,
    paragraph: RendererParagraph,
    selection: Option<Selection>,
    dragging: bool,
    focused: bool,
    hovered_link: Option<usize>,
    pressed_link: Option<usize>,
    previous_click: Option<mouse::Click>,
}

impl<Message> Widget<Message, Theme, iced::Renderer> for SelectableRichText<Message>
where
    Message: 'static,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<SelectableRichTextState>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(SelectableRichTextState::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<SelectableRichTextState>();
        layout::sized(limits, self.width, Length::Shrink, |limits| {
            let bounds = limits.max();
            let size = self.size.unwrap_or_else(|| renderer.default_size());
            let spans = split_graphemes(&self.spans);
            let text = || Text {
                content: spans.as_slice(),
                bounds,
                size,
                line_height: self.line_height,
                font: renderer.default_font(),
                align_x: self.align_x,
                align_y: iced::alignment::Vertical::Top,
                shaping: Shaping::Advanced,
                wrapping: Wrapping::default(),
                ellipsis: Ellipsis::default(),
                hint_factor: renderer.hint_factor(),
            };
            if state.spans != spans {
                state.paragraph = RendererParagraph::with_spans(text());
                state.spans = spans;
                state.selection = None;
            } else {
                match state.paragraph.compare(text().with_content(())) {
                    iced::advanced::text::Difference::None => {}
                    iced::advanced::text::Difference::Bounds => state.paragraph.resize(bounds),
                    iced::advanced::text::Difference::Shape => {
                        state.paragraph = RendererParagraph::with_spans(text());
                    }
                }
            }
            state.paragraph.min_bounds()
        })
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if !layout.bounds().intersects(viewport) {
            return;
        }
        let state = tree.state.downcast_ref::<SelectableRichTextState>();
        let translation = layout.position() - Point::ORIGIN;

        if let Some(selection) = state.selection.filter(|selection| !selection.is_empty()) {
            let (start, end) = selection.ordered();
            for index in start..end.min(state.spans.len()) {
                for bounds in state.paragraph.span_bounds(index) {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: bounds + translation,
                            ..renderer::Quad::default()
                        },
                        self.selection_color,
                    );
                }
            }
        }

        for (index, span) in state.spans.iter().enumerate() {
            let hovered_link = self.on_link_click.is_some()
                && state.hovered_link == Some(index)
                && span.link.is_some();
            if span.highlight.is_none() && !span.underline && !span.strikethrough && !hovered_link {
                continue;
            }
            let regions = state.paragraph.span_bounds(index);
            if let Some(highlight) = span.highlight {
                for bounds in &regions {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                bounds.position() + translation
                                    - Vector::new(span.padding.left, span.padding.top),
                                bounds.size() + Size::new(span.padding.x(), span.padding.y()),
                            ),
                            border: highlight.border,
                            ..renderer::Quad::default()
                        },
                        highlight.background,
                    );
                }
            }
            if span.underline || span.strikethrough || hovered_link {
                let size = span.size.or(self.size).unwrap_or(renderer.default_size());
                let line_height = span
                    .line_height
                    .unwrap_or(self.line_height)
                    .to_absolute(size);
                let color = span.color.or(self.color).unwrap_or(defaults.text_color);
                let baseline =
                    translation + Vector::new(0.0, size.0 + (line_height.0 - size.0) / 2.0);
                if span.underline || hovered_link {
                    for bounds in &regions {
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: Rectangle::new(
                                    bounds.position() + baseline - Vector::new(0.0, size.0 * 0.08),
                                    Size::new(bounds.width, 1.0),
                                ),
                                ..renderer::Quad::default()
                            },
                            color,
                        );
                    }
                }
                if span.strikethrough {
                    for bounds in &regions {
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: Rectangle::new(
                                    bounds.position() + baseline - Vector::new(0.0, size.0 / 2.0),
                                    Size::new(bounds.width, 1.0),
                                ),
                                ..renderer::Quad::default()
                            },
                            color,
                        );
                    }
                }
            }
        }

        text::draw(
            renderer,
            defaults,
            layout.bounds(),
            &state.paragraph,
            Style { color: self.color },
            viewport,
        );
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<SelectableRichTextState>();
        let local = cursor.position_in(layout.bounds());
        state.hovered_link = local
            .and_then(|position| state.paragraph.hit_span(position))
            .filter(|index| {
                state
                    .spans
                    .get(*index)
                    .is_some_and(|span| span.link.is_some())
            });

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(position) = cursor.position() else {
                    return;
                };
                if !layout.bounds().contains(position) {
                    state.focused = false;
                    state.selection = None;
                    state.dragging = false;
                    state.pressed_link = None;
                    return;
                }
                let Some(index) = local.and_then(|position| state.paragraph.hit_span(position))
                else {
                    return;
                };
                state.focused = true;
                state.dragging = true;
                state.pressed_link = state.hovered_link;
                let click = mouse::Click::new(position, mouse::Button::Left, state.previous_click);
                state.previous_click = Some(click);
                state.selection = Some(match click.kind() {
                    mouse::click::Kind::Single => Selection {
                        anchor: index,
                        focus: index,
                    },
                    mouse::click::Kind::Double => select_word(&state.spans, index),
                    mouse::click::Kind::Triple => Selection {
                        anchor: 0,
                        focus: state.spans.len(),
                    },
                });
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                if let Some(index) = local.and_then(|position| state.paragraph.hit_span(position))
                    && let Some(selection) = state.selection.as_mut()
                {
                    selection.focus = if index >= selection.anchor {
                        index.saturating_add(1)
                    } else {
                        index
                    };
                    state.pressed_link = None;
                    shell.capture_event();
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                if let Some(index) = state.pressed_link.take()
                    && state.hovered_link == Some(index)
                    && state.selection.is_some_and(Selection::is_empty)
                    && let Some(link) = state.spans.get(index).and_then(|span| span.link.clone())
                    && let Some(on_link_click) = &self.on_link_click
                {
                    shell.publish(on_link_click(link));
                }
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                if state.focused && modifiers.command() =>
            {
                match key.as_ref() {
                    keyboard::Key::Character("c") => {
                        if let Some(value) = selected_text(state) {
                            shell.write_clipboard(clipboard::Content::Text(value));
                            shell.capture_event();
                        }
                    }
                    keyboard::Key::Character("a") => {
                        state.selection = Some(Selection {
                            anchor: 0,
                            focus: state.spans.len(),
                        });
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    _ => {}
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) if state.focused => {
                state.selection = None;
                state.focused = false;
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Window(iced::window::Event::Unfocused) => {
                state.dragging = false;
                state.pressed_link = None;
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<SelectableRichTextState>();
        if state.hovered_link.is_some() {
            mouse::Interaction::Pointer
        } else if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }
}

impl<Message> From<SelectableRichText<Message>> for Element<'static, Message>
where
    Message: 'static,
{
    fn from(value: SelectableRichText<Message>) -> Self {
        Element::new(value)
    }
}

fn split_graphemes(spans: &[Span<'static, String, Font>]) -> Vec<Span<'static, String, Font>> {
    let mut split = Vec::new();
    for span in spans {
        for grapheme in span.text.graphemes(true) {
            let mut next = span.clone();
            next.text = Cow::Owned(grapheme.to_owned());
            split.push(next);
        }
    }
    split
}

fn select_word(spans: &[Span<'static, String, Font>], index: usize) -> Selection {
    let is_boundary = |span: &Span<'static, String, Font>| {
        span.text.chars().all(char::is_whitespace)
            || span.text.chars().all(|value| value.is_ascii_punctuation())
    };
    if spans.get(index).is_none_or(is_boundary) {
        return Selection {
            anchor: index,
            focus: index.saturating_add(1).min(spans.len()),
        };
    }
    let mut start = index;
    while start > 0 && !is_boundary(&spans[start - 1]) {
        start -= 1;
    }
    let mut end = index.saturating_add(1);
    while end < spans.len() && !is_boundary(&spans[end]) {
        end += 1;
    }
    Selection {
        anchor: start,
        focus: end,
    }
}

fn selected_text(state: &SelectableRichTextState) -> Option<String> {
    let selection = state.selection.filter(|selection| !selection.is_empty())?;
    let (start, end) = selection.ordered();
    Some(
        state.spans[start..end.min(state.spans.len())]
            .iter()
            .map(|span| span.text.as_ref())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(value: &str) -> Vec<Span<'static, String, Font>> {
        vec![Span::new(value.to_owned())]
    }

    #[test]
    fn split_preserves_unicode_graphemes_and_copy_order() {
        let spans = split_graphemes(&spans("A你e\u{301}"));
        assert_eq!(spans.len(), 3);
        let state = SelectableRichTextState {
            spans,
            selection: Some(Selection {
                anchor: 3,
                focus: 1,
            }),
            ..SelectableRichTextState::default()
        };
        assert_eq!(selected_text(&state).as_deref(), Some("你e\u{301}"));
    }

    #[test]
    fn double_click_word_selection_stops_at_whitespace_and_punctuation() {
        let spans = split_graphemes(&spans("one two,three"));
        let selection = select_word(&spans, 5);
        let state = SelectableRichTextState {
            spans,
            selection: Some(selection),
            ..SelectableRichTextState::default()
        };
        assert_eq!(selected_text(&state).as_deref(), Some("two"));
    }
}
