use iced::keyboard::key::Named;
use iced::widget::canvas;
use iced::{
    Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, keyboard, mouse, touch,
};

use super::ControlSize;
use crate::theme::ThemeTokens;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct XYPadValue {
    pub x: f32,
    pub y: f32,
}

impl XYPadValue {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum XYPadEvent {
    Input(XYPadValue),
    Change(XYPadValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxisLock {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Interaction {
    None,
    Mouse,
    Touch(touch::Finger),
}

#[derive(Debug)]
pub struct XYPadState {
    focused: bool,
    interaction: Interaction,
    current: Option<XYPadValue>,
    origin: Point,
    axis_lock: Option<AxisLock>,
    modifiers: keyboard::Modifiers,
}

impl Default for XYPadState {
    fn default() -> Self {
        Self {
            focused: false,
            interaction: Interaction::None,
            current: None,
            origin: Point::ORIGIN,
            axis_lock: None,
            modifiers: keyboard::Modifiers::default(),
        }
    }
}

/// A native two-axis control with live pointer input and committed keyboard changes.
///
/// Pointer and touch input publish [`XYPadEvent::Input`] while dragging and
/// [`XYPadEvent::Change`] on release. Arrow keys commit changes after the pad
/// receives focus from a primary click. Holding Shift during a mouse drag locks
/// movement to the first dominant axis.
pub struct XYPad<'a, Message> {
    value: XYPadValue,
    on_event: Box<dyn Fn(XYPadEvent) -> Message + 'a>,
    x_min: f32,
    x_max: f32,
    y_min: f32,
    y_max: f32,
    step: f32,
    size: ControlSize,
    disabled: bool,
    loading: bool,
    invalid: bool,
    tokens: ThemeTokens,
}

impl<'a, Message> XYPad<'a, Message>
where
    Message: 'a,
{
    pub fn new(
        value: XYPadValue,
        on_event: impl Fn(XYPadEvent) -> Message + 'a,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            value,
            on_event: Box::new(on_event),
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.0,
            y_max: 1.0,
            step: 0.0,
            size: ControlSize::Medium,
            disabled: false,
            loading: false,
            invalid: false,
            tokens: theme.into(),
        }
    }

    pub fn x_range(mut self, min: f32, max: f32) -> Self {
        (self.x_min, self.x_max) = valid_range(min, max);
        self
    }

    pub fn y_range(mut self, min: f32, max: f32) -> Self {
        (self.y_min, self.y_max) = valid_range(min, max);
        self
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = if step.is_finite() && step > 0.0 {
            step
        } else {
            0.0
        };
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        let height = match self.size {
            ControlSize::Small => 40.0,
            ControlSize::Medium => 48.0,
            ControlSize::Large => 64.0,
        };
        canvas(self)
            .width(Length::Fill)
            .height(Length::Fixed(height))
            .into()
    }

    fn inactive(&self) -> bool {
        self.disabled || self.loading
    }

    fn displayed_value(&self, state: &XYPadState) -> XYPadValue {
        state.current.unwrap_or(self.value)
    }

    fn value_at(
        &self,
        point: Point,
        bounds: Size,
        locked: Option<(AxisLock, XYPadValue)>,
    ) -> XYPadValue {
        let nx = if bounds.width > 0.0 {
            (point.x / bounds.width).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let ny = if bounds.height > 0.0 {
            (point.y / bounds.height).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let mut value = XYPadValue {
            x: quantize(
                self.x_min + nx * (self.x_max - self.x_min),
                self.x_min,
                self.x_max,
                self.step,
            ),
            y: quantize(
                self.y_max - ny * (self.y_max - self.y_min),
                self.y_min,
                self.y_max,
                self.step,
            ),
        };
        if let Some((axis, previous)) = locked {
            match axis {
                AxisLock::Horizontal => value.y = previous.y,
                AxisLock::Vertical => value.x = previous.x,
            }
        }
        value
    }

    fn keyboard_value(&self, value: XYPadValue, key: Named) -> Option<XYPadValue> {
        let delta = if self.step > 0.0 {
            self.step
        } else {
            (self.x_max - self.x_min)
                .max(self.y_max - self.y_min)
                .max(1.0)
                / 100.0
        };
        let mut next = value;
        match key {
            Named::ArrowLeft => next.x -= delta,
            Named::ArrowRight => next.x += delta,
            Named::ArrowUp => next.y += delta,
            Named::ArrowDown => next.y -= delta,
            _ => return None,
        }
        next.x = quantize(next.x, self.x_min, self.x_max, self.step);
        next.y = quantize(next.y, self.y_min, self.y_max, self.step);
        Some(next)
    }
}

impl<Message> canvas::Program<Message> for XYPad<'_, Message> {
    type State = XYPadState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            state.modifiers = *modifiers;
            return None;
        }
        if let canvas::Event::Window(iced::window::Event::Unfocused) = event {
            state.focused = false;
            state.interaction = Interaction::None;
            state.current = None;
            state.axis_lock = None;
            return Some(canvas::Action::request_redraw());
        }
        if self.inactive() {
            state.interaction = Interaction::None;
            state.current = None;
            return None;
        }

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(point) = cursor.position_in(bounds) else {
                    if state.focused {
                        state.focused = false;
                        return Some(canvas::Action::request_redraw());
                    }
                    return None;
                };
                state.focused = true;
                state.interaction = Interaction::Mouse;
                state.origin = point;
                state.axis_lock = None;
                let value = self.value_at(point, bounds.size(), None);
                state.current = Some(value);
                Some(
                    canvas::Action::publish((self.on_event)(XYPadEvent::Input(value)))
                        .and_capture(),
                )
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. })
                if state.interaction == Interaction::Mouse =>
            {
                let absolute = cursor.position()?;
                let point = Point::new(absolute.x - bounds.x, absolute.y - bounds.y);
                if state.modifiers.shift() && state.axis_lock.is_none() {
                    let dx = (point.x - state.origin.x).abs() / bounds.width.max(1.0);
                    let dy = (point.y - state.origin.y).abs() / bounds.height.max(1.0);
                    state.axis_lock = Some(if dx >= dy {
                        AxisLock::Horizontal
                    } else {
                        AxisLock::Vertical
                    });
                } else if !state.modifiers.shift() {
                    state.axis_lock = None;
                }
                let previous = state.current.unwrap_or(self.value);
                let value = self.value_at(
                    point,
                    bounds.size(),
                    state.axis_lock.map(|axis| (axis, previous)),
                );
                state.current = Some(value);
                Some(
                    canvas::Action::publish((self.on_event)(XYPadEvent::Input(value)))
                        .and_capture(),
                )
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.interaction == Interaction::Mouse =>
            {
                let value = cursor
                    .position()
                    .map(|absolute| Point::new(absolute.x - bounds.x, absolute.y - bounds.y))
                    .map(|point| {
                        let previous = state.current.unwrap_or(self.value);
                        self.value_at(
                            point,
                            bounds.size(),
                            state.axis_lock.map(|axis| (axis, previous)),
                        )
                    })
                    .unwrap_or_else(|| state.current.unwrap_or(self.value));
                state.interaction = Interaction::None;
                state.current = None;
                state.axis_lock = None;
                Some(
                    canvas::Action::publish((self.on_event)(XYPadEvent::Change(value)))
                        .and_capture(),
                )
            }
            canvas::Event::Touch(touch::Event::FingerPressed { id, position })
                if bounds.contains(*position) =>
            {
                let point = Point::new(position.x - bounds.x, position.y - bounds.y);
                state.interaction = Interaction::Touch(*id);
                state.origin = point;
                state.axis_lock = None;
                let value = self.value_at(point, bounds.size(), None);
                state.current = Some(value);
                Some(
                    canvas::Action::publish((self.on_event)(XYPadEvent::Input(value)))
                        .and_capture(),
                )
            }
            canvas::Event::Touch(touch::Event::FingerMoved { id, position })
                if state.interaction == Interaction::Touch(*id) =>
            {
                let point = Point::new(position.x - bounds.x, position.y - bounds.y);
                let value = self.value_at(point, bounds.size(), None);
                state.current = Some(value);
                Some(
                    canvas::Action::publish((self.on_event)(XYPadEvent::Input(value)))
                        .and_capture(),
                )
            }
            canvas::Event::Touch(
                touch::Event::FingerLifted { id, position }
                | touch::Event::FingerLost { id, position },
            ) if state.interaction == Interaction::Touch(*id) => {
                let point = Point::new(position.x - bounds.x, position.y - bounds.y);
                let value = self.value_at(point, bounds.size(), None);
                state.interaction = Interaction::None;
                state.current = None;
                Some(
                    canvas::Action::publish((self.on_event)(XYPadEvent::Change(value)))
                        .and_capture(),
                )
            }
            canvas::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(key),
                ..
            }) if state.focused => {
                self.keyboard_value(self.displayed_value(state), *key)
                    .map(|value| {
                        canvas::Action::publish((self.on_event)(XYPadEvent::Change(value)))
                            .and_capture()
                    })
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
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let colors = self.tokens.colors;
        let metrics = self.tokens.metrics;
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let surface =
            canvas::Path::rounded_rectangle(Point::ORIGIN, bounds.size(), metrics.radius_xs.into());
        let opacity = if self.inactive() { 0.5 } else { 1.0 };
        frame.fill(&surface, with_alpha(colors.subtle, opacity));
        let border = if self.invalid {
            colors.danger
        } else if state.focused {
            colors.accent
        } else {
            colors.border
        };
        frame.stroke(
            &surface,
            canvas::Stroke::default()
                .with_color(with_alpha(border, opacity))
                .with_width(if state.focused { 2.0 } else { 1.0 }),
        );

        let crosshair = with_alpha(colors.border_strong, 0.55 * opacity);
        let vertical = canvas::Path::line(
            Point::new(bounds.width / 2.0, 0.0),
            Point::new(bounds.width / 2.0, bounds.height),
        );
        let horizontal = canvas::Path::line(
            Point::new(0.0, bounds.height / 2.0),
            Point::new(bounds.width, bounds.height / 2.0),
        );
        frame.stroke(&vertical, canvas::Stroke::default().with_color(crosshair));
        frame.stroke(&horizontal, canvas::Stroke::default().with_color(crosshair));

        let value = self.displayed_value(state);
        let x = axis_fraction(value.x, self.x_min, self.x_max) * bounds.width;
        let y = (1.0 - axis_fraction(value.y, self.y_min, self.y_max)) * bounds.height;
        let radius = match self.size {
            ControlSize::Small => 4.0,
            ControlSize::Medium => 5.0,
            ControlSize::Large => 6.0,
        };
        let thumb = canvas::Path::circle(Point::new(x, y), radius);
        frame.fill(&thumb, with_alpha(colors.accent, 0.72 * opacity));
        frame.stroke(
            &thumb,
            canvas::Stroke::default()
                .with_color(with_alpha(
                    if self.invalid {
                        colors.danger
                    } else {
                        colors.accent
                    },
                    opacity,
                ))
                .with_width(1.0),
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if !cursor.is_over(bounds) {
            mouse::Interaction::None
        } else if self.inactive() {
            mouse::Interaction::NotAllowed
        } else if state.interaction == Interaction::Mouse {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Crosshair
        }
    }
}

fn valid_range(min: f32, max: f32) -> (f32, f32) {
    if min.is_finite() && max.is_finite() && max > min {
        (min, max)
    } else {
        (0.0, 1.0)
    }
}

fn quantize(value: f32, min: f32, max: f32, step: f32) -> f32 {
    let value = value.clamp(min, max);
    if step <= 0.0 || step.is_nan() {
        return value;
    }
    let stepped = ((value - min) / step).round() * step + min;
    ((stepped * 1_000_000.0).round() / 1_000_000.0).clamp(min, max)
}

fn axis_fraction(value: f32, min: f32, max: f32) -> f32 {
    ((value.clamp(min, max) - min) / (max - min)).clamp(0.0, 1.0)
}

fn with_alpha(color: Color, multiplier: f32) -> Color {
    Color {
        a: color.a * multiplier,
        ..color
    }
}

#[cfg(test)]
mod tests {
    use super::{XYPad, XYPadEvent, XYPadValue};
    use crate::ThemeMode;
    use iced::{Point, Size, keyboard::key::Named};

    fn pad(value: XYPadValue) -> XYPad<'static, XYPadEvent> {
        XYPad::new(value, |event| event, ThemeMode::Dark.tokens())
    }

    #[test]
    fn pointer_geometry_inverts_y_and_quantizes_both_axes() {
        let pad = pad(XYPadValue::default())
            .x_range(-1.0, 1.0)
            .y_range(0.0, 10.0)
            .step(0.25);
        assert_eq!(
            pad.value_at(Point::new(75.0, 25.0), Size::new(100.0, 100.0), None),
            XYPadValue::new(0.5, 7.5)
        );
        assert_eq!(
            pad.value_at(Point::new(200.0, -10.0), Size::new(100.0, 100.0), None),
            XYPadValue::new(1.0, 10.0)
        );
    }

    #[test]
    fn keyboard_changes_use_step_and_clamp_to_axis_range() {
        let pad = pad(XYPadValue::new(0.9, 0.0)).step(0.25);
        assert_eq!(
            pad.keyboard_value(XYPadValue::new(0.9, 0.0), Named::ArrowRight),
            Some(XYPadValue::new(1.0, 0.0))
        );
        assert_eq!(
            pad.keyboard_value(XYPadValue::new(0.5, 0.5), Named::ArrowUp),
            Some(XYPadValue::new(0.5, 0.75))
        );
    }
}
