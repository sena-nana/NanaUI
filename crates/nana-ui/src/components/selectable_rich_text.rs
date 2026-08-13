use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

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
    selection_group: Option<(TextSelectionGroup, u64, String)>,
    on_selection_change: Option<Box<dyn Fn(Option<TextSelectionSnapshot>) -> Message>>,
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
            selection_group: None,
            on_selection_change: None,
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

    /// Joins this text node to a retained document-level selection.
    ///
    /// `order` must be stable and unique within the group. `separator_before`
    /// is inserted before this node when a selection spans a preceding node.
    pub fn selection_group(
        mut self,
        group: TextSelectionGroup,
        order: u64,
        separator_before: impl Into<String>,
    ) -> Self {
        self.selection_group = Some((group, order, separator_before.into()));
        self
    }

    pub fn on_selection_change(
        mut self,
        on_selection_change: impl Fn(Option<String>) -> Message + 'static,
    ) -> Self {
        self.on_selection_change = Some(Box::new(move |selection| {
            on_selection_change(selection.map(|selection| selection.text))
        }));
        self
    }

    /// Reports the selected text together with its surface-relative visual bounds.
    ///
    /// Use this for selection toolbars and other overlays that must not change
    /// the document layout. The bounds are absent only before the selected
    /// nodes have produced geometry.
    pub fn on_selection_snapshot(
        mut self,
        on_selection_change: impl Fn(Option<TextSelectionSnapshot>) -> Message + 'static,
    ) -> Self {
        self.on_selection_change = Some(Box::new(on_selection_change));
        self
    }
}

/// A completed rich-text selection and its surface-relative visual envelope.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSelectionSnapshot {
    pub text: String,
    pub bounds: Option<Rectangle>,
}

/// Shared selection state for a document composed from multiple rich-text nodes.
///
/// This keeps selection in the document model instead of in an application-owned
/// collection of rendered widgets. It enables continuous drag selection, copy,
/// and selection-driven actions across Markdown paragraphs, lists, quotes, code,
/// and table cells.
#[derive(Clone, Debug, Default)]
pub struct TextSelectionGroup {
    state: Arc<Mutex<TextSelectionGroupState>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct GroupSelectionPoint {
    order: u64,
    index: usize,
}

#[derive(Clone, Debug, Default)]
struct TextSelectionNode {
    graphemes: Vec<String>,
    separator_before: String,
    bounds: Rectangle,
    span_bounds: Vec<Vec<Rectangle>>,
}

#[derive(Debug, Default)]
struct TextSelectionGroupState {
    nodes: BTreeMap<u64, TextSelectionNode>,
    anchor: Option<GroupSelectionPoint>,
    focus: Option<GroupSelectionPoint>,
    dragging_owner: Option<u64>,
}

impl TextSelectionGroup {
    pub fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.anchor = None;
            state.focus = None;
            state.dragging_owner = None;
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.state
            .lock()
            .ok()
            .and_then(|state| group_selected_text(&state))
    }

    pub fn selection_snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.state
            .lock()
            .ok()
            .and_then(|state| group_selection_snapshot(&state))
    }

    fn register_text(
        &self,
        order: u64,
        spans: &[Span<'static, String, Font>],
        separator_before: &str,
    ) {
        let graphemes = spans
            .iter()
            .map(|span| span.text.as_ref().to_owned())
            .collect::<Vec<_>>();
        if let Ok(mut state) = self.state.lock() {
            let node = state.nodes.entry(order).or_default();
            node.graphemes = graphemes;
            node.separator_before.clear();
            node.separator_before.push_str(separator_before);
            normalize_group_selection(&mut state);
        }
    }

    fn register_geometry(&self, order: u64, bounds: Rectangle, span_bounds: Vec<Vec<Rectangle>>) {
        if let Ok(mut state) = self.state.lock()
            && let Some(node) = state.nodes.get_mut(&order)
        {
            node.bounds = bounds;
            node.span_bounds = span_bounds;
        }
    }

    fn selection_for(&self, order: u64) -> Option<Selection> {
        self.state
            .lock()
            .ok()
            .and_then(|state| group_selection_for(&state, order))
    }

    fn begin(&self, order: u64, selection: Selection) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let Some(node) = state.nodes.get(&order) else {
            return false;
        };
        let length = node.graphemes.len();
        state.anchor = Some(GroupSelectionPoint {
            order,
            index: selection.anchor.min(length),
        });
        state.focus = Some(GroupSelectionPoint {
            order,
            index: selection.focus.min(length),
        });
        state.dragging_owner = Some(order);
        true
    }

    fn drag(&self, owner: u64, position: Point) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.dragging_owner != Some(owner) {
            return false;
        }
        let Some(point) = group_hit_point(&state, position) else {
            return false;
        };
        if state.focus == Some(point) {
            return false;
        }
        state.focus = Some(point);
        true
    }

    fn finish(&self, owner: u64) -> Option<Option<TextSelectionSnapshot>> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        if state.dragging_owner != Some(owner) {
            return None;
        }
        state.dragging_owner = None;
        Some(group_selection_snapshot(&state))
    }

    fn select_all(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let Some((&first_order, _)) = state.nodes.first_key_value() else {
            return false;
        };
        let Some((&last_order, last)) = state.nodes.last_key_value() else {
            return false;
        };
        let last_length = last.graphemes.len();
        state.anchor = Some(GroupSelectionPoint {
            order: first_order,
            index: 0,
        });
        state.focus = Some(GroupSelectionPoint {
            order: last_order,
            index: last_length,
        });
        true
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
            if let Some((group, order, separator_before)) = &self.selection_group {
                group.register_text(*order, &state.spans, separator_before);
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
        if let Some((group, order, _)) = &self.selection_group {
            group.register_geometry(
                *order,
                layout.bounds(),
                (0..state.spans.len())
                    .map(|index| {
                        state
                            .paragraph
                            .span_bounds(index)
                            .into_iter()
                            .map(|bounds| bounds + translation)
                            .collect()
                    })
                    .collect(),
            );
        }

        let selection = self
            .selection_group
            .as_ref()
            .and_then(|(group, order, _)| group.selection_for(*order))
            .or(state.selection)
            .filter(|selection| !selection.is_empty());
        if let Some(selection) = selection {
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
                let selection = match click.kind() {
                    mouse::click::Kind::Single => Selection {
                        anchor: index,
                        focus: index,
                    },
                    mouse::click::Kind::Double => select_word(&state.spans, index),
                    mouse::click::Kind::Triple => Selection {
                        anchor: 0,
                        focus: state.spans.len(),
                    },
                };
                if let Some((group, order, _)) = &self.selection_group {
                    group.begin(*order, selection);
                    state.selection = None;
                } else {
                    state.selection = Some(selection);
                }
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                if let Some((group, order, _)) = &self.selection_group {
                    if let Some(position) = cursor.position()
                        && group.drag(*order, position)
                    {
                        state.pressed_link = None;
                        shell.capture_event();
                        shell.request_redraw();
                    }
                } else if let Some(index) =
                    local.and_then(|position| state.paragraph.hit_span(position))
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
                let grouped_selection = self
                    .selection_group
                    .as_ref()
                    .and_then(|(group, order, _)| group.finish(*order));
                if let Some(selected) = grouped_selection.as_ref()
                    && let Some(on_selection_change) = &self.on_selection_change
                {
                    shell.publish(on_selection_change(selected.clone()));
                }
                if let Some(index) = state.pressed_link.take()
                    && state.hovered_link == Some(index)
                    && grouped_selection.as_ref().map_or_else(
                        || state.selection.is_some_and(Selection::is_empty),
                        Option::is_none,
                    )
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
                        let value = self
                            .selection_group
                            .as_ref()
                            .and_then(|(group, _, _)| group.selected_text())
                            .or_else(|| selected_text(state));
                        if let Some(value) = value {
                            shell.write_clipboard(clipboard::Content::Text(value));
                            shell.capture_event();
                        }
                    }
                    keyboard::Key::Character("a") => {
                        if let Some((group, _, _)) = &self.selection_group {
                            group.select_all();
                            state.selection = None;
                        } else {
                            state.selection = Some(Selection {
                                anchor: 0,
                                focus: state.spans.len(),
                            });
                        }
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
                if let Some((group, _, _)) = &self.selection_group {
                    group.clear();
                    if let Some(on_selection_change) = &self.on_selection_change {
                        shell.publish(on_selection_change(None));
                    }
                } else {
                    state.selection = None;
                }
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

fn normalize_group_selection(state: &mut TextSelectionGroupState) {
    let normalize = |point: GroupSelectionPoint| {
        state
            .nodes
            .get(&point.order)
            .map(|node| GroupSelectionPoint {
                order: point.order,
                index: point.index.min(node.graphemes.len()),
            })
    };
    state.anchor = state.anchor.and_then(normalize);
    state.focus = state.focus.and_then(normalize);
    if state.anchor.is_none() || state.focus.is_none() {
        state.anchor = None;
        state.focus = None;
        state.dragging_owner = None;
    }
}

fn ordered_group_selection(
    state: &TextSelectionGroupState,
) -> Option<(GroupSelectionPoint, GroupSelectionPoint)> {
    let anchor = state.anchor?;
    let focus = state.focus?;
    if anchor <= focus {
        Some((anchor, focus))
    } else {
        Some((focus, anchor))
    }
}

fn group_selection_for(state: &TextSelectionGroupState, order: u64) -> Option<Selection> {
    let (start, end) = ordered_group_selection(state)?;
    if start == end || order < start.order || order > end.order {
        return None;
    }
    let length = state.nodes.get(&order)?.graphemes.len();
    let local_start = if order == start.order { start.index } else { 0 }.min(length);
    let local_end = if order == end.order {
        end.index
    } else {
        length
    }
    .min(length);
    (local_start != local_end).then_some(Selection {
        anchor: local_start,
        focus: local_end,
    })
}

fn group_selected_text(state: &TextSelectionGroupState) -> Option<String> {
    let (start, end) = ordered_group_selection(state)?;
    if start == end {
        return None;
    }
    let mut value = String::new();
    for (&order, node) in state.nodes.range(start.order..=end.order) {
        let local_start =
            if order == start.order { start.index } else { 0 }.min(node.graphemes.len());
        let local_end = if order == end.order {
            end.index
        } else {
            node.graphemes.len()
        }
        .min(node.graphemes.len());
        if local_start >= local_end {
            continue;
        }
        if !value.is_empty() {
            value.push_str(&node.separator_before);
        }
        value.extend(
            node.graphemes[local_start..local_end]
                .iter()
                .map(String::as_str),
        );
    }
    (!value.is_empty()).then_some(value)
}

fn group_selection_snapshot(state: &TextSelectionGroupState) -> Option<TextSelectionSnapshot> {
    let text = group_selected_text(state)?;
    let (start, end) = ordered_group_selection(state)?;
    let mut bounds = None;
    for (&order, node) in state.nodes.range(start.order..=end.order) {
        let local_start =
            if order == start.order { start.index } else { 0 }.min(node.graphemes.len());
        let local_end = if order == end.order {
            end.index
        } else {
            node.graphemes.len()
        }
        .min(node.graphemes.len());
        let geometry_start = local_start.min(node.span_bounds.len());
        let geometry_end = local_end.min(node.span_bounds.len());
        for selected in node.span_bounds[geometry_start..geometry_end]
            .iter()
            .flatten()
            .copied()
        {
            bounds = Some(match bounds {
                Some(current) => rectangle_union(current, selected),
                None => selected,
            });
        }
    }
    Some(TextSelectionSnapshot { text, bounds })
}

fn rectangle_union(left: Rectangle, right: Rectangle) -> Rectangle {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (left.x + left.width).max(right.x + right.width);
    let bottom_edge = (left.y + left.height).max(right.y + right.height);
    Rectangle::new(Point::new(x, y), Size::new(right_edge - x, bottom_edge - y))
}

fn group_hit_point(
    state: &TextSelectionGroupState,
    position: Point,
) -> Option<GroupSelectionPoint> {
    let (&order, node) = state.nodes.iter().min_by(|(_, left), (_, right)| {
        point_rectangle_distance_squared(position, left.bounds)
            .total_cmp(&point_rectangle_distance_squared(position, right.bounds))
    })?;
    let length = node.graphemes.len();
    let first_top = node
        .span_bounds
        .iter()
        .flatten()
        .map(|bounds| bounds.y)
        .min_by(f32::total_cmp);
    let last_bottom = node
        .span_bounds
        .iter()
        .flatten()
        .map(|bounds| bounds.y + bounds.height)
        .max_by(f32::total_cmp);
    if first_top.is_some_and(|top| position.y < top) {
        return Some(GroupSelectionPoint { order, index: 0 });
    }
    if last_bottom.is_some_and(|bottom| position.y > bottom) {
        return Some(GroupSelectionPoint {
            order,
            index: length,
        });
    }
    let (index, bounds) = node
        .span_bounds
        .iter()
        .enumerate()
        .flat_map(|(index, bounds)| bounds.iter().map(move |bounds| (index, *bounds)))
        .min_by(|(_, left), (_, right)| {
            point_rectangle_distance_squared(position, *left)
                .total_cmp(&point_rectangle_distance_squared(position, *right))
        })?;
    Some(GroupSelectionPoint {
        order,
        index: if position.x >= bounds.x + bounds.width / 2.0 {
            index.saturating_add(1).min(length)
        } else {
            index
        },
    })
}

fn point_rectangle_distance_squared(point: Point, bounds: Rectangle) -> f32 {
    let dx = if point.x < bounds.x {
        bounds.x - point.x
    } else if point.x > bounds.x + bounds.width {
        point.x - (bounds.x + bounds.width)
    } else {
        0.0
    };
    let dy = if point.y < bounds.y {
        bounds.y - point.y
    } else if point.y > bounds.y + bounds.height {
        point.y - (bounds.y + bounds.height)
    } else {
        0.0
    };
    dx.mul_add(dx, dy * dy)
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

    #[test]
    fn selection_group_preserves_document_order_and_block_separators() {
        let group = TextSelectionGroup::default();
        group.register_text(10, &split_graphemes(&spans("Alpha")), "");
        group.register_text(20, &split_graphemes(&spans("Beta")), "\n\n");
        assert!(group.begin(
            10,
            Selection {
                anchor: 2,
                focus: 5,
            },
        ));
        {
            let mut state = group.state.lock().unwrap();
            state.focus = Some(GroupSelectionPoint {
                order: 20,
                index: 2,
            });
        }

        assert_eq!(group.selected_text().as_deref(), Some("pha\n\nBe"));
        assert_eq!(
            group.selection_for(10),
            Some(Selection {
                anchor: 2,
                focus: 5,
            })
        );
        assert_eq!(
            group.selection_for(20),
            Some(Selection {
                anchor: 0,
                focus: 2,
            })
        );
    }

    #[test]
    fn selection_snapshot_unions_only_selected_grapheme_bounds() {
        let group = TextSelectionGroup::default();
        group.register_text(10, &split_graphemes(&spans("ABC")), "");
        group.register_geometry(
            10,
            Rectangle::new(Point::new(10.0, 20.0), Size::new(90.0, 40.0)),
            vec![
                vec![Rectangle::new(
                    Point::new(10.0, 20.0),
                    Size::new(10.0, 10.0),
                )],
                vec![Rectangle::new(
                    Point::new(20.0, 20.0),
                    Size::new(10.0, 10.0),
                )],
                vec![Rectangle::new(
                    Point::new(10.0, 40.0),
                    Size::new(10.0, 10.0),
                )],
            ],
        );
        assert!(group.begin(
            10,
            Selection {
                anchor: 1,
                focus: 3,
            },
        ));
        let snapshot = group.selection_snapshot().unwrap();
        assert_eq!(snapshot.text, "BC");
        assert_eq!(
            snapshot.bounds,
            Some(Rectangle::new(
                Point::new(10.0, 20.0),
                Size::new(20.0, 30.0),
            ))
        );
    }
}
