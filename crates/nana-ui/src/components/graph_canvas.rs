use std::borrow::Cow;

use iced::keyboard::key::Named;
use iced::widget::canvas;
use iced::{
    Color, Element, Length, Pixels, Point, Rectangle, Renderer, Size, Theme, alignment, font,
    keyboard, mouse, touch,
};

use crate::graph::{
    GraphCanvasId, GraphEdge, GraphEndpoint, GraphModel, GraphNodeId, GraphPoint, GraphPortKind,
    GraphPortSide, GraphSelection, GraphSize, GraphTargetDescriptor, GraphViewport, cubic_point,
    port_tangent,
};
use crate::theme::{ThemeTokens, ui_font};

pub use nana_ui_runtime::GraphCanvasEvent;

const DEFAULT_GRID_SPACING: f32 = 24.0;
const KEYBOARD_PAN_STEP: f32 = 32.0;
const KEYBOARD_ZOOM_FACTOR: f32 = 1.2;

#[derive(Debug, Clone, Default, PartialEq)]
enum GraphInteraction {
    #[default]
    None,
    MousePan {
        origin: Point,
        viewport: GraphViewport,
    },
    TouchPan {
        finger: touch::Finger,
        origin: Point,
        viewport: GraphViewport,
    },
    MouseNodeDrag {
        node: GraphNodeId,
        origin: Point,
        node_origin: GraphPoint,
        current: GraphPoint,
    },
    MouseConnection {
        source: GraphEndpoint,
        current: GraphPoint,
    },
}

#[derive(Debug, Default)]
pub struct GraphCanvasState {
    focused: bool,
    interaction: GraphInteraction,
    preview_viewport: Option<GraphViewport>,
}

/// A controlled native graph canvas for node workflows and IDE-style tools.
///
/// The application owns the graph, viewport and selection. NanaUI owns only
/// transient pointer state and publishes typed events. Stable sub-targets for
/// accessibility and debug adapters are available through
/// [`GraphCanvas::target_descriptors`].
pub struct GraphCanvas<'a, Message> {
    canvas_id: GraphCanvasId,
    model: Cow<'a, GraphModel>,
    viewport: GraphViewport,
    selection: Option<Cow<'a, GraphSelection>>,
    on_event: Box<dyn Fn(GraphCanvasEvent) -> Message + 'a>,
    tokens: ThemeTokens,
    width: Length,
    height: Length,
    grid_spacing: f32,
    disabled: bool,
}

impl<'a, Message> GraphCanvas<'a, Message>
where
    Message: 'a,
{
    pub fn new(
        canvas_id: impl Into<GraphCanvasId>,
        model: &'a GraphModel,
        viewport: GraphViewport,
        selection: Option<&'a GraphSelection>,
        on_event: impl Fn(GraphCanvasEvent) -> Message + 'a,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            canvas_id: canvas_id.into(),
            model: Cow::Borrowed(model),
            viewport,
            selection: selection.map(Cow::Borrowed),
            on_event: Box::new(on_event),
            tokens: theme.into(),
            width: Length::Fill,
            height: Length::Fill,
            grid_spacing: DEFAULT_GRID_SPACING,
            disabled: false,
        }
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn grid_spacing(mut self, spacing: f32) -> Self {
        if spacing.is_finite() && spacing >= 8.0 {
            self.grid_spacing = spacing;
        }
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn target_descriptors(&self, canvas_size: GraphSize) -> Vec<GraphTargetDescriptor> {
        self.model.as_ref().target_descriptors(
            &self.canvas_id,
            self.viewport,
            self.selection.as_deref(),
            canvas_size,
        )
    }

    pub fn view(self) -> Element<'a, Message> {
        let width = self.width;
        let height = self.height;
        canvas(self).width(width).height(height).into()
    }

    fn displayed_viewport(&self, state: &GraphCanvasState) -> GraphViewport {
        state.preview_viewport.unwrap_or(self.viewport)
    }

    fn local_cursor(bounds: Rectangle, cursor: mouse::Cursor) -> Option<GraphPoint> {
        cursor
            .position_in(bounds)
            .map(|position| GraphPoint::new(position.x, position.y))
    }

    fn begin_mouse_pan(&self, state: &mut GraphCanvasState, cursor: mouse::Cursor) -> Option<()> {
        let origin = cursor.position()?;
        state.interaction = GraphInteraction::MousePan {
            origin,
            viewport: self.displayed_viewport(state),
        };
        Some(())
    }

    fn begin_touch_pan(
        &self,
        state: &mut GraphCanvasState,
        finger: touch::Finger,
        position: Point,
    ) {
        state.interaction = GraphInteraction::TouchPan {
            finger,
            origin: position,
            viewport: self.displayed_viewport(state),
        };
    }

    fn publish(&self, event: GraphCanvasEvent) -> canvas::Action<Message> {
        canvas::Action::publish((self.on_event)(event)).and_capture()
    }
}

impl<Message> GraphCanvas<'static, Message>
where
    Message: 'static,
{
    pub fn owned(
        canvas_id: impl Into<GraphCanvasId>,
        model: GraphModel,
        viewport: GraphViewport,
        selection: Option<GraphSelection>,
        on_event: impl Fn(GraphCanvasEvent) -> Message + 'static,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            canvas_id: canvas_id.into(),
            model: Cow::Owned(model),
            viewport,
            selection: selection.map(Cow::Owned),
            on_event: Box::new(on_event),
            tokens: theme.into(),
            width: Length::Fill,
            height: Length::Fill,
            grid_spacing: DEFAULT_GRID_SPACING,
            disabled: false,
        }
    }
}

impl<'a, Message: 'a> canvas::Program<Message> for GraphCanvas<'a, Message> {
    type State = GraphCanvasState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if state.preview_viewport == Some(self.viewport)
            && matches!(&state.interaction, GraphInteraction::None)
        {
            state.preview_viewport = None;
        }

        if let canvas::Event::Window(iced::window::Event::Unfocused) = event {
            state.focused = false;
            state.interaction = GraphInteraction::None;
            state.preview_viewport = None;
            return Some(canvas::Action::request_redraw());
        }
        if self.disabled {
            state.interaction = GraphInteraction::None;
            state.preview_viewport = None;
            return None;
        }

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(position) = Self::local_cursor(bounds, cursor) else {
                    if state.focused {
                        state.focused = false;
                        return Some(canvas::Action::request_redraw());
                    }
                    return None;
                };
                state.focused = true;
                if let Some(selection) = self
                    .model
                    .hit_test(self.displayed_viewport(state), position)
                {
                    state.interaction = match &selection {
                        GraphSelection::Node(node) => self
                            .model
                            .node(node)
                            .and_then(|graph_node| {
                                cursor
                                    .position()
                                    .map(|origin| GraphInteraction::MouseNodeDrag {
                                        node: node.clone(),
                                        origin,
                                        node_origin: graph_node.position,
                                        current: graph_node.position,
                                    })
                            })
                            .unwrap_or_default(),
                        GraphSelection::Port { node, port } => GraphInteraction::MouseConnection {
                            source: GraphEndpoint::new(node.clone(), port.clone()),
                            current: position,
                        },
                        GraphSelection::Edge(_) => GraphInteraction::None,
                    };
                    return Some(self.publish(GraphCanvasEvent::SelectionChanged(Some(selection))));
                }
                self.begin_mouse_pan(state, cursor)?;
                Some(self.publish(GraphCanvasEvent::SelectionChanged(None)))
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle))
                if cursor.is_over(bounds) =>
            {
                state.focused = true;
                self.begin_mouse_pan(state, cursor)?;
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                match state.interaction.clone() {
                    GraphInteraction::MousePan { origin, viewport } => {
                        let next = viewport.pan_by(position.x - origin.x, position.y - origin.y);
                        state.preview_viewport = Some(next);
                        Some(self.publish(GraphCanvasEvent::ViewportInput(next)))
                    }
                    GraphInteraction::MouseNodeDrag {
                        node,
                        origin,
                        node_origin,
                        ..
                    } => {
                        let viewport = self.displayed_viewport(state);
                        let next = GraphPoint::new(
                            node_origin.x + (position.x - origin.x) / viewport.zoom,
                            node_origin.y + (position.y - origin.y) / viewport.zoom,
                        );
                        if let GraphInteraction::MouseNodeDrag { current, .. } =
                            &mut state.interaction
                        {
                            *current = next;
                        }
                        Some(self.publish(GraphCanvasEvent::NodePositionInput {
                            node,
                            position: next,
                        }))
                    }
                    GraphInteraction::MouseConnection { .. } => {
                        let local = GraphPoint::new(position.x - bounds.x, position.y - bounds.y);
                        if let GraphInteraction::MouseConnection { current, .. } =
                            &mut state.interaction
                        {
                            *current = local;
                        }
                        Some(canvas::Action::request_redraw().and_capture())
                    }
                    _ => None,
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left | mouse::Button::Middle,
            )) => {
                let interaction = std::mem::take(&mut state.interaction);
                match interaction {
                    GraphInteraction::MousePan { .. } => {
                        let viewport = self.displayed_viewport(state);
                        Some(self.publish(GraphCanvasEvent::ViewportChanged(viewport)))
                    }
                    GraphInteraction::MouseNodeDrag { node, current, .. } => {
                        Some(self.publish(GraphCanvasEvent::NodePositionChanged {
                            node,
                            position: current,
                        }))
                    }
                    GraphInteraction::MouseConnection { source, .. } => {
                        let target = Self::local_cursor(bounds, cursor).and_then(|position| {
                            match self
                                .model
                                .hit_test(self.displayed_viewport(state), position)
                            {
                                Some(GraphSelection::Port { node, port }) => order_connection(
                                    self.model.as_ref(),
                                    source.clone(),
                                    GraphEndpoint::new(node, port),
                                ),
                                Some(GraphSelection::Node(node)) => {
                                    self.model.node(&node).and_then(|graph_node| {
                                        graph_node.ports.iter().find_map(|port| {
                                            order_connection(
                                                self.model.as_ref(),
                                                source.clone(),
                                                GraphEndpoint::new(node.clone(), port.id.clone()),
                                            )
                                        })
                                    })
                                }
                                _ => None,
                            }
                        });
                        target.map(|(source, target)| {
                            self.publish(GraphCanvasEvent::ConnectionRequested { source, target })
                        })
                    }
                    _ => None,
                }
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor.is_over(bounds) =>
            {
                let anchor = Self::local_cursor(bounds, cursor)?;
                let factor = wheel_zoom_factor(*delta);
                let next = self.displayed_viewport(state).zoom_at(anchor, factor);
                state.preview_viewport = Some(next);
                state.focused = true;
                Some(self.publish(GraphCanvasEvent::ViewportChanged(next)))
            }
            canvas::Event::Touch(touch::Event::FingerPressed { id, position })
                if bounds.contains(*position) =>
            {
                state.focused = true;
                let local = GraphPoint::new(position.x - bounds.x, position.y - bounds.y);
                if let Some(selection) = self.model.hit_test(self.displayed_viewport(state), local)
                {
                    state.interaction = GraphInteraction::None;
                    return Some(self.publish(GraphCanvasEvent::SelectionChanged(Some(selection))));
                }
                self.begin_touch_pan(state, *id, *position);
                Some(self.publish(GraphCanvasEvent::SelectionChanged(None)))
            }
            canvas::Event::Touch(touch::Event::FingerMoved { id, position }) => {
                let GraphInteraction::TouchPan {
                    finger,
                    origin,
                    viewport,
                } = state.interaction.clone()
                else {
                    return None;
                };
                if finger != *id {
                    return None;
                }
                let next = viewport.pan_by(position.x - origin.x, position.y - origin.y);
                state.preview_viewport = Some(next);
                Some(self.publish(GraphCanvasEvent::ViewportInput(next)))
            }
            canvas::Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) => {
                let GraphInteraction::TouchPan { finger, .. } = state.interaction.clone() else {
                    return None;
                };
                if finger != *id {
                    return None;
                }
                state.interaction = GraphInteraction::None;
                let viewport = self.displayed_viewport(state);
                Some(self.publish(GraphCanvasEvent::ViewportChanged(viewport)))
            }
            canvas::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) if state.focused => {
                let viewport = self.displayed_viewport(state);
                let center = GraphPoint::new(bounds.width * 0.5, bounds.height * 0.5);
                let next = match key.as_ref() {
                    keyboard::Key::Named(Named::ArrowLeft) => {
                        Some(viewport.pan_by(KEYBOARD_PAN_STEP, 0.0))
                    }
                    keyboard::Key::Named(Named::ArrowRight) => {
                        Some(viewport.pan_by(-KEYBOARD_PAN_STEP, 0.0))
                    }
                    keyboard::Key::Named(Named::ArrowUp) => {
                        Some(viewport.pan_by(0.0, KEYBOARD_PAN_STEP))
                    }
                    keyboard::Key::Named(Named::ArrowDown) => {
                        Some(viewport.pan_by(0.0, -KEYBOARD_PAN_STEP))
                    }
                    keyboard::Key::Named(Named::ZoomIn)
                    | keyboard::Key::Character("+")
                    | keyboard::Key::Character("=") => {
                        Some(viewport.zoom_at(center, KEYBOARD_ZOOM_FACTOR))
                    }
                    keyboard::Key::Named(Named::ZoomOut) | keyboard::Key::Character("-") => {
                        Some(viewport.zoom_at(center, 1.0 / KEYBOARD_ZOOM_FACTOR))
                    }
                    keyboard::Key::Named(Named::Home) | keyboard::Key::Character("0") => self
                        .model
                        .bounds()
                        .map(|graph_bounds| {
                            GraphViewport::fit(
                                graph_bounds,
                                GraphSize::new(bounds.width, bounds.height),
                                36.0,
                            )
                        })
                        .or(Some(GraphViewport::default())),
                    keyboard::Key::Named(Named::Escape) => {
                        return Some(self.publish(GraphCanvasEvent::SelectionChanged(None)));
                    }
                    _ => None,
                }?;
                state.preview_viewport = Some(next);
                Some(self.publish(GraphCanvasEvent::ViewportChanged(next)))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let viewport = self.displayed_viewport(state);
        let hover = Self::local_cursor(bounds, cursor)
            .and_then(|position| self.model.hit_test(viewport, position));
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let opacity = if self.disabled { 0.5 } else { 1.0 };

        let background = canvas::Path::rectangle(Point::ORIGIN, bounds.size());
        frame.fill(
            &background,
            with_alpha(self.tokens.colors.background, opacity),
        );
        draw_grid(
            &mut frame,
            viewport,
            self.grid_spacing,
            self.tokens,
            opacity,
        );
        for edge in self.model.edges() {
            draw_edge(
                &mut frame,
                self.model.as_ref(),
                edge,
                viewport,
                self.selection.as_deref(),
                hover.as_ref(),
                self.tokens,
                opacity,
            );
        }
        if let GraphInteraction::MouseConnection { source, current } = &state.interaction
            && let Some(origin) = self.model.port_position(source)
            && let Some(side) = graph_port_side(self.model.as_ref(), &source.node, &source.port)
        {
            draw_pending_connection(
                &mut frame,
                viewport.world_to_view(origin),
                *current,
                side,
                self.tokens,
                opacity,
            );
        }
        for node in self.model.nodes() {
            draw_node(
                &mut frame,
                self.model.as_ref(),
                node,
                viewport,
                self.selection.as_deref(),
                hover.as_ref(),
                self.tokens,
                opacity,
                bounds.size(),
            );
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if !cursor.is_over(bounds) {
            return mouse::Interaction::None;
        }
        if self.disabled {
            return mouse::Interaction::NotAllowed;
        }
        if matches!(&state.interaction, GraphInteraction::MousePan { .. }) {
            return mouse::Interaction::Grabbing;
        }
        let viewport = self.displayed_viewport(state);
        if Self::local_cursor(bounds, cursor)
            .and_then(|position| self.model.hit_test(viewport, position))
            .is_some()
        {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::Grab
        }
    }
}

fn wheel_zoom_factor(delta: mouse::ScrollDelta) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => KEYBOARD_ZOOM_FACTOR.powf(y),
        mouse::ScrollDelta::Pixels { y, .. } => (y * 0.0025).exp(),
    }
}

fn draw_grid(
    frame: &mut canvas::Frame,
    viewport: GraphViewport,
    base_spacing: f32,
    tokens: ThemeTokens,
    opacity: f32,
) {
    let mut spacing = base_spacing * viewport.zoom;
    while spacing < 16.0 {
        spacing *= 2.0;
    }
    while spacing > 96.0 {
        spacing *= 0.5;
    }
    let color = with_alpha(tokens.colors.border_soft, 0.72 * opacity);
    let x_start = viewport.offset.x.rem_euclid(spacing);
    let y_start = viewport.offset.y.rem_euclid(spacing);
    let mut x = x_start;
    while x <= frame.width() {
        frame.stroke(
            &canvas::Path::line(Point::new(x, 0.0), Point::new(x, frame.height())),
            canvas::Stroke::default().with_color(color).with_width(1.0),
        );
        x += spacing;
    }
    let mut y = y_start;
    while y <= frame.height() {
        frame.stroke(
            &canvas::Path::line(Point::new(0.0, y), Point::new(frame.width(), y)),
            canvas::Stroke::default().with_color(color).with_width(1.0),
        );
        y += spacing;
    }
}

fn order_connection(
    model: &GraphModel,
    source: GraphEndpoint,
    target: GraphEndpoint,
) -> Option<(GraphEndpoint, GraphEndpoint)> {
    if source == target {
        return None;
    }
    let source_kind = graph_port_kind(model, &source.node, &source.port)?;
    let target_kind = graph_port_kind(model, &target.node, &target.port)?;
    let source_out = matches!(
        source_kind,
        GraphPortKind::Output | GraphPortKind::Bidirectional
    );
    let source_in = matches!(
        source_kind,
        GraphPortKind::Input | GraphPortKind::Bidirectional
    );
    let target_out = matches!(
        target_kind,
        GraphPortKind::Output | GraphPortKind::Bidirectional
    );
    let target_in = matches!(
        target_kind,
        GraphPortKind::Input | GraphPortKind::Bidirectional
    );
    if source_out && target_in {
        Some((source, target))
    } else if source_in && target_out {
        Some((target, source))
    } else {
        None
    }
}

fn graph_port_kind(
    model: &GraphModel,
    node_id: &GraphNodeId,
    port_id: &crate::graph::GraphPortId,
) -> Option<GraphPortKind> {
    model
        .node(node_id)?
        .ports
        .iter()
        .find(|port| &port.id == port_id)
        .map(|port| port.kind)
}

fn graph_port_side(
    model: &GraphModel,
    node_id: &GraphNodeId,
    port_id: &crate::graph::GraphPortId,
) -> Option<GraphPortSide> {
    model
        .node(node_id)?
        .ports
        .iter()
        .find(|port| &port.id == port_id)
        .map(|port| port.side)
}

fn draw_pending_connection(
    frame: &mut canvas::Frame,
    source: GraphPoint,
    target: GraphPoint,
    source_side: GraphPortSide,
    tokens: ThemeTokens,
    opacity: f32,
) {
    let reach =
        ((target.x - source.x).abs().max((target.y - source.y).abs()) * 0.45).clamp(32.0, 180.0);
    let out = port_tangent(source_side);
    let incoming = Point::new(target.x - out.x * reach, target.y - out.y * reach);
    let path = canvas::Path::new(|builder| {
        builder.move_to(to_iced_point(source));
        builder.bezier_curve_to(
            Point::new(source.x + out.x * reach, source.y + out.y * reach),
            incoming,
            to_iced_point(target),
        );
    });
    frame.stroke(
        &path,
        canvas::Stroke::default()
            .with_color(with_alpha(tokens.colors.accent, opacity * 0.8))
            .with_width(1.8),
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_edge(
    frame: &mut canvas::Frame,
    model: &GraphModel,
    edge: &GraphEdge,
    viewport: GraphViewport,
    selection: Option<&GraphSelection>,
    hover: Option<&GraphSelection>,
    tokens: ThemeTokens,
    opacity: f32,
) {
    let Some(curve) = model.edge_curve(edge, viewport) else {
        return;
    };
    let selected = selection == Some(&GraphSelection::Edge(edge.id.clone()));
    let hovered = hover == Some(&GraphSelection::Edge(edge.id.clone()));
    let color = if selected {
        tokens.colors.text
    } else if hovered {
        tokens.colors.muted
    } else {
        tokens.colors.border_strong
    };
    let path = canvas::Path::new(|builder| {
        builder.move_to(to_iced_point(curve[0]));
        builder.bezier_curve_to(
            to_iced_point(curve[1]),
            to_iced_point(curve[2]),
            to_iced_point(curve[3]),
        );
    });
    frame.stroke(
        &path,
        canvas::Stroke::default()
            .with_color(with_alpha(color, opacity))
            .with_width(1.6),
    );

    if viewport.zoom >= 0.7
        && let Some(label) = edge.label.as_ref()
    {
        let center = cubic_point(curve, 0.5);
        frame.fill_text(canvas::Text {
            content: label.clone(),
            position: Point::new(center.x, center.y - 6.0),
            color: with_alpha(tokens.colors.muted, opacity),
            size: Pixels(10.0),
            font: ui_font(font::Weight::Normal),
            align_x: alignment::Horizontal::Center.into(),
            align_y: alignment::Vertical::Bottom,
            ..canvas::Text::default()
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    frame: &mut canvas::Frame,
    model: &GraphModel,
    node: &crate::graph::GraphNode,
    viewport: GraphViewport,
    selection: Option<&GraphSelection>,
    hover: Option<&GraphSelection>,
    tokens: ThemeTokens,
    opacity: f32,
    canvas_size: Size,
) {
    let bounds = viewport.world_rect_to_view(node.bounds());
    if bounds.max_x() < 0.0
        || bounds.max_y() < 0.0
        || bounds.origin.x > canvas_size.width
        || bounds.origin.y > canvas_size.height
    {
        return;
    }
    let selected = selection == Some(&GraphSelection::Node(node.id.clone()));
    let hovered = hover == Some(&GraphSelection::Node(node.id.clone()));
    let fill = if selected {
        tokens.colors.selected
    } else if hovered {
        tokens.colors.hover
    } else {
        tokens.colors.surface
    };
    let path = canvas::Path::rounded_rectangle(
        to_iced_point(bounds.origin),
        Size::new(bounds.size.width, bounds.size.height),
        (tokens.metrics.radius_sm * viewport.zoom.clamp(0.5, 1.0)).into(),
    );
    frame.fill(&path, with_alpha(fill, opacity));
    frame.stroke(
        &path,
        canvas::Stroke::default()
            .with_color(with_alpha(
                if selected {
                    tokens.colors.border_strong
                } else {
                    tokens.colors.border
                },
                opacity,
            ))
            .with_width(if selected { 1.5 } else { 1.0 }),
    );

    let title_height = (28.0 * viewport.zoom).clamp(18.0, 34.0);
    if bounds.size.width >= 32.0 && bounds.size.height >= title_height {
        let separator_y = bounds.origin.y + title_height;
        frame.stroke(
            &canvas::Path::line(
                Point::new(bounds.origin.x, separator_y),
                Point::new(bounds.max_x(), separator_y),
            ),
            canvas::Stroke::default()
                .with_color(with_alpha(tokens.colors.border_soft, opacity))
                .with_width(1.0),
        );
        let text_size = (12.0 * viewport.zoom).clamp(9.0, 13.0);
        frame.fill_text(canvas::Text {
            content: truncate_label(&node.label, bounds.size.width - 20.0, text_size),
            position: Point::new(bounds.origin.x + 10.0, bounds.origin.y + title_height * 0.5),
            max_width: (bounds.size.width - 20.0).max(1.0),
            color: with_alpha(tokens.colors.text, opacity),
            size: Pixels(text_size),
            font: ui_font(font::Weight::Medium),
            align_y: alignment::Vertical::Center,
            ..canvas::Text::default()
        });
    }

    for port in &node.ports {
        let endpoint = GraphEndpoint::new(node.id.clone(), port.id.clone());
        let Some(world_position) = model.port_position(&endpoint) else {
            continue;
        };
        let position = viewport.world_to_view(world_position);
        let port_selection = GraphSelection::Port {
            node: node.id.clone(),
            port: port.id.clone(),
        };
        let port_selected = selection == Some(&port_selection);
        let port_hovered = hover == Some(&port_selection);
        let radius = if port_selected || port_hovered {
            5.0
        } else {
            4.0
        };
        let handle = canvas::Path::circle(to_iced_point(position), radius);
        let handle_color = match port.kind {
            GraphPortKind::Input => tokens.colors.muted,
            GraphPortKind::Output => tokens.colors.accent,
            GraphPortKind::Bidirectional => tokens.colors.warning,
        };
        frame.fill(&handle, with_alpha(tokens.colors.background, opacity));
        frame.stroke(
            &handle,
            canvas::Stroke::default()
                .with_color(with_alpha(handle_color, opacity))
                .with_width(if port_selected { 2.4 } else { 1.6 }),
        );
        if viewport.zoom >= 0.72 && !port.label.is_empty() {
            draw_port_label(frame, position, port.side, &port.label, tokens, opacity);
        }
    }
}

fn draw_port_label(
    frame: &mut canvas::Frame,
    position: GraphPoint,
    side: GraphPortSide,
    label: &str,
    tokens: ThemeTokens,
    opacity: f32,
) {
    let (position, align_x, align_y) = match side {
        GraphPortSide::Top => (
            Point::new(position.x, position.y + 8.0),
            alignment::Horizontal::Center,
            alignment::Vertical::Top,
        ),
        GraphPortSide::Right => (
            Point::new(position.x - 9.0, position.y),
            alignment::Horizontal::Right,
            alignment::Vertical::Center,
        ),
        GraphPortSide::Bottom => (
            Point::new(position.x, position.y - 8.0),
            alignment::Horizontal::Center,
            alignment::Vertical::Bottom,
        ),
        GraphPortSide::Left => (
            Point::new(position.x + 9.0, position.y),
            alignment::Horizontal::Left,
            alignment::Vertical::Center,
        ),
    };
    frame.fill_text(canvas::Text {
        content: truncate_label(label, 80.0, 9.5),
        position,
        max_width: 80.0,
        color: with_alpha(tokens.colors.muted, opacity),
        size: Pixels(9.5),
        font: ui_font(font::Weight::Normal),
        align_x: align_x.into(),
        align_y,
        ..canvas::Text::default()
    });
}

fn truncate_label(label: &str, max_width: f32, text_size: f32) -> String {
    if max_width <= 0.0 {
        return String::new();
    }
    let max_chars = (max_width / (text_size * 0.62)).floor().max(1.0) as usize;
    if label.chars().count() <= max_chars {
        return label.to_owned();
    }
    let visible = max_chars.saturating_sub(1);
    let mut truncated = label.chars().take(visible).collect::<String>();
    truncated.push('…');
    truncated
}

fn to_iced_point(point: GraphPoint) -> Point {
    Point::new(point.x, point.y)
}

fn with_alpha(color: Color, multiplier: f32) -> Color {
    Color {
        a: color.a * multiplier,
        ..color
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_zoom_is_smooth_for_lines_and_pixels() {
        assert!(
            (wheel_zoom_factor(mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 }) - 1.2).abs() < 0.001
        );
        let pixel = wheel_zoom_factor(mouse::ScrollDelta::Pixels { x: 0.0, y: 100.0 });
        assert!(pixel > 1.0 && pixel < 2.0);
    }

    #[test]
    fn long_labels_are_truncated_without_breaking_unicode() {
        let label = truncate_label("输入节点 Alpha", 40.0, 10.0);
        assert!(label.ends_with('…'));
        assert!(label.is_char_boundary(label.len()));
    }
}
