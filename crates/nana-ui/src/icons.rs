use iced::widget::canvas::{Path, Stroke};
use iced::widget::{canvas, container, space};
use iced::{Border, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};
use nana_ui_scene::{IconPathCommand, IconShape, icon_geometry};

pub use nana_ui_core::Icon;

#[derive(Debug, Clone, Copy)]
struct LineIcon {
    icon: Icon,
    color: Color,
}

#[derive(Debug, Clone, Copy)]
struct DisclosureIcon {
    expansion: f32,
    color: Color,
}

#[derive(Debug, Clone, Copy)]
struct SpinnerIcon {
    phase: u8,
    color: Color,
}

impl<Message> canvas::Program<Message> for LineIcon {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let scale = bounds.width.min(bounds.height) / 24.0;
        let offset = Point::new(
            (bounds.width - 24.0 * scale) / 2.0,
            (bounds.height - 24.0 * scale) / 2.0,
        );

        paint_icon_geometry(&mut frame, self.icon, scale, offset, self.color, 1.7);

        vec![frame.into_geometry()]
    }
}

pub(crate) fn paint_icon_geometry(
    frame: &mut canvas::Frame<Renderer>,
    icon: Icon,
    scale: f32,
    offset: Point,
    color: Color,
    stroke_width: f32,
) {
    let stroke = Stroke::default()
        .with_color(color)
        .with_width(stroke_width * scale);
    for shape in icon_geometry(icon).shapes {
        let (path, filled) = match shape {
            IconShape::Path(commands) => (
                Path::new(|builder| {
                    for command in commands {
                        match command {
                            IconPathCommand::MoveTo([x, y]) => {
                                builder.move_to(point(scale, offset, x, y));
                            }
                            IconPathCommand::LineTo([x, y]) => {
                                builder.line_to(point(scale, offset, x, y));
                            }
                            IconPathCommand::CubicTo {
                                control_a: [ax, ay],
                                control_b: [bx, by],
                                to: [x, y],
                            } => builder.bezier_curve_to(
                                point(scale, offset, ax, ay),
                                point(scale, offset, bx, by),
                                point(scale, offset, x, y),
                            ),
                            IconPathCommand::Close => builder.close(),
                        }
                    }
                }),
                false,
            ),
            IconShape::Circle {
                center: [x, y],
                radius,
            } => (
                Path::circle(point(scale, offset, x, y), radius * scale),
                false,
            ),
            IconShape::Rect {
                origin: [x, y],
                size: [width, height],
                filled,
            } => (
                Path::rectangle(
                    point(scale, offset, x, y),
                    Size::new(width * scale, height * scale),
                ),
                filled,
            ),
            IconShape::RoundedRect {
                origin: [x, y],
                size: [width, height],
                radius,
            } => (
                Path::rounded_rectangle(
                    point(scale, offset, x, y),
                    Size::new(width * scale, height * scale),
                    iced::border::Radius::from(radius * scale),
                ),
                false,
            ),
        };
        if filled {
            frame.fill(&path, color);
        } else {
            frame.stroke(&path, stroke);
        }
    }
}

impl<Message> canvas::Program<Message> for SpinnerIcon {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        paint_spinner_geometry(&mut frame, bounds.size(), self.phase, self.color);
        vec![frame.into_geometry()]
    }
}

pub(crate) fn paint_spinner_geometry(
    frame: &mut canvas::Frame<Renderer>,
    size: Size,
    phase: u8,
    color: Color,
) {
    let scale = size.width.min(size.height) / 24.0;
    let center = Point::new(size.width / 2.0, size.height / 2.0);
    for index in 0..8_u8 {
        let angle = f32::from(index) * std::f32::consts::FRAC_PI_4;
        let from = Point::new(
            center.x + angle.cos() * 6.0 * scale,
            center.y + angle.sin() * 6.0 * scale,
        );
        let to = Point::new(
            center.x + angle.cos() * 10.0 * scale,
            center.y + angle.sin() * 10.0 * scale,
        );
        let distance = (index + 8 - phase % 8) % 8;
        let alpha = 1.0 - f32::from(distance) * 0.105;
        frame.stroke(
            &Path::line(from, to),
            Stroke::default()
                .with_color(Color {
                    a: color.a * alpha,
                    ..color
                })
                .with_width(2.2 * scale),
        );
    }
}

impl<Message> canvas::Program<Message> for DisclosureIcon {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let scale = bounds.width.min(bounds.height) / 24.0;
        let offset = Point::new(
            (bounds.width - 24.0 * scale) / 2.0,
            (bounds.height - 24.0 * scale) / 2.0,
        );
        let angle = self.expansion.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
        let rotate = |x: f32, y: f32| {
            let x = x - 12.0;
            let y = y - 12.0;
            (
                12.0 + x * angle.cos() - y * angle.sin(),
                12.0 + x * angle.sin() + y * angle.cos(),
            )
        };
        let start = rotate(9.0, 6.0);
        let middle = rotate(15.0, 12.0);
        let end = rotate(9.0, 18.0);
        let path = Path::new(|builder| {
            builder.move_to(point(scale, offset, start.0, start.1));
            builder.line_to(point(scale, offset, middle.0, middle.1));
            builder.line_to(point(scale, offset, end.0, end.1));
        });
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(self.color)
                .with_width(1.8 * scale),
        );
        vec![frame.into_geometry()]
    }
}

fn point(scale: f32, offset: Point, x: f32, y: f32) -> Point {
    Point::new(offset.x + x * scale, offset.y + y * scale)
}

pub fn icon<'a, Message: 'a>(kind: Icon, size: f32, color: Color) -> Element<'a, Message> {
    container(canvas(LineIcon { icon: kind, color }))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

/// A right-to-down chevron driven by a normalized expansion value.
pub fn disclosure_icon<'a, Message: 'a>(
    expansion: f32,
    size: f32,
    color: Color,
) -> Element<'a, Message> {
    container(canvas(DisclosureIcon { expansion, color }))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

/// A baseline-independent filled or outlined status dot.
pub fn status_indicator<'a, Message: 'a>(
    filled: bool,
    size: f32,
    color: Color,
) -> Element<'a, Message> {
    let diameter = size * 10.0 / 24.0;
    let dot = container(space())
        .width(Length::Fixed(diameter))
        .height(Length::Fixed(diameter))
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(if filled { color } else { Color::TRANSPARENT })
                .border(
                    Border::default()
                        .rounded(999.0)
                        .width(if filled { 0.0 } else { 1.0 })
                        .color(color),
                )
        });
    container(dot)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

/// A centered eight-segment activity indicator.
pub fn spinner_icon<'a, Message: 'a>(phase: u8, size: f32, color: Color) -> Element<'a, Message> {
    container(canvas(SpinnerIcon { phase, color }))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}
