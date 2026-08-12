use iced::widget::canvas::{Path, Stroke};
use iced::widget::{canvas, container, space};
use iced::{Border, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};

/// Compact line icons used by NanaUI navigation surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    About,
    Add,
    Appearance,
    ArrowLeft,
    Chart,
    Close,
    Eye,
    File,
    Folder,
    Maximize,
    Minimize,
    Moon,
    Nodes,
    Restore,
    Search,
    Settings,
    Sidebar,
    Workspace,
}

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

impl LineIcon {
    fn stroke(self, frame: &mut canvas::Frame<Renderer>, path: &Path, scale: f32) {
        frame.stroke(
            path,
            Stroke::default()
                .with_color(self.color)
                .with_width(1.7 * scale),
        );
    }

    fn line(
        self,
        frame: &mut canvas::Frame<Renderer>,
        scale: f32,
        offset: Point,
        from: (f32, f32),
        to: (f32, f32),
    ) {
        self.stroke(
            frame,
            &Path::line(
                point(scale, offset, from.0, from.1),
                point(scale, offset, to.0, to.1),
            ),
            scale,
        );
    }

    fn circle(
        self,
        frame: &mut canvas::Frame<Renderer>,
        scale: f32,
        offset: Point,
        center: (f32, f32),
        radius: f32,
    ) {
        self.stroke(
            frame,
            &Path::circle(point(scale, offset, center.0, center.1), radius * scale),
            scale,
        );
    }
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

        match self.icon {
            Icon::About => {
                self.circle(&mut frame, scale, offset, (12.0, 12.0), 9.0);
                self.circle(&mut frame, scale, offset, (12.0, 8.0), 0.45);
                self.line(&mut frame, scale, offset, (12.0, 11.0), (12.0, 16.0));
            }
            Icon::Add => {
                let vertical = Path::rectangle(
                    point(scale, offset, 10.8, 4.0),
                    Size::new(2.4 * scale, 16.0 * scale),
                );
                let horizontal = Path::rectangle(
                    point(scale, offset, 4.0, 10.8),
                    Size::new(16.0 * scale, 2.4 * scale),
                );
                frame.fill(&vertical, self.color);
                frame.fill(&horizontal, self.color);
            }
            Icon::Appearance => {
                self.circle(&mut frame, scale, offset, (12.0, 12.0), 4.0);
                for (from, to) in [
                    ((12.0, 2.0), (12.0, 5.0)),
                    ((12.0, 19.0), (12.0, 22.0)),
                    ((2.0, 12.0), (5.0, 12.0)),
                    ((19.0, 12.0), (22.0, 12.0)),
                    ((4.9, 4.9), (7.0, 7.0)),
                    ((17.0, 17.0), (19.1, 19.1)),
                    ((4.9, 19.1), (7.0, 17.0)),
                    ((17.0, 7.0), (19.1, 4.9)),
                ] {
                    self.line(&mut frame, scale, offset, from, to);
                }
            }
            Icon::ArrowLeft => {
                self.line(&mut frame, scale, offset, (20.0, 12.0), (5.0, 12.0));
                self.line(&mut frame, scale, offset, (11.0, 6.0), (5.0, 12.0));
                self.line(&mut frame, scale, offset, (5.0, 12.0), (11.0, 18.0));
            }
            Icon::Chart => {
                self.line(&mut frame, scale, offset, (4.0, 20.0), (4.0, 4.0));
                self.line(&mut frame, scale, offset, (4.0, 20.0), (21.0, 20.0));
                let series = Path::new(|builder| {
                    builder.move_to(point(scale, offset, 6.0, 16.0));
                    builder.line_to(point(scale, offset, 10.0, 11.0));
                    builder.line_to(point(scale, offset, 14.0, 14.0));
                    builder.line_to(point(scale, offset, 20.0, 6.0));
                });
                self.stroke(&mut frame, &series, scale);
            }
            Icon::Close => {
                let close = Path::new(|builder| {
                    builder.move_to(point(scale, offset, 5.0, 5.0));
                    builder.line_to(point(scale, offset, 19.0, 19.0));
                    builder.move_to(point(scale, offset, 19.0, 5.0));
                    builder.line_to(point(scale, offset, 5.0, 19.0));
                });
                self.stroke(&mut frame, &close, scale);
            }
            Icon::Eye => {
                let outline = Path::new(|builder| {
                    builder.move_to(point(scale, offset, 2.5, 12.0));
                    builder.bezier_curve_to(
                        point(scale, offset, 6.0, 6.5),
                        point(scale, offset, 9.0, 5.0),
                        point(scale, offset, 12.0, 5.0),
                    );
                    builder.bezier_curve_to(
                        point(scale, offset, 15.0, 5.0),
                        point(scale, offset, 18.0, 6.5),
                        point(scale, offset, 21.5, 12.0),
                    );
                    builder.bezier_curve_to(
                        point(scale, offset, 18.0, 17.5),
                        point(scale, offset, 15.0, 19.0),
                        point(scale, offset, 12.0, 19.0),
                    );
                    builder.bezier_curve_to(
                        point(scale, offset, 9.0, 19.0),
                        point(scale, offset, 6.0, 17.5),
                        point(scale, offset, 2.5, 12.0),
                    );
                });
                self.stroke(&mut frame, &outline, scale);
                self.circle(&mut frame, scale, offset, (12.0, 12.0), 2.5);
            }
            Icon::File => {
                let file = Path::new(|builder| {
                    builder.move_to(point(scale, offset, 6.0, 3.0));
                    builder.line_to(point(scale, offset, 14.0, 3.0));
                    builder.line_to(point(scale, offset, 19.0, 8.0));
                    builder.line_to(point(scale, offset, 19.0, 21.0));
                    builder.line_to(point(scale, offset, 6.0, 21.0));
                    builder.close();
                    builder.move_to(point(scale, offset, 14.0, 3.0));
                    builder.line_to(point(scale, offset, 14.0, 8.0));
                    builder.line_to(point(scale, offset, 19.0, 8.0));
                });
                self.stroke(&mut frame, &file, scale);
            }
            Icon::Folder => {
                let folder = Path::new(|builder| {
                    builder.move_to(point(scale, offset, 3.0, 6.0));
                    builder.line_to(point(scale, offset, 9.0, 6.0));
                    builder.line_to(point(scale, offset, 11.0, 9.0));
                    builder.line_to(point(scale, offset, 21.0, 9.0));
                    builder.line_to(point(scale, offset, 21.0, 20.0));
                    builder.line_to(point(scale, offset, 3.0, 20.0));
                    builder.close();
                });
                self.stroke(&mut frame, &folder, scale);
            }
            Icon::Maximize => {
                let square = Path::rectangle(
                    point(scale, offset, 5.0, 5.0),
                    Size::new(14.0 * scale, 14.0 * scale),
                );
                self.stroke(&mut frame, &square, scale);
            }
            Icon::Minimize => {
                self.line(&mut frame, scale, offset, (5.0, 12.0), (19.0, 12.0));
            }
            Icon::Moon => {
                let moon = Path::new(|builder| {
                    builder.move_to(point(scale, offset, 17.5, 3.5));
                    builder.bezier_curve_to(
                        point(scale, offset, 12.7, 4.1),
                        point(scale, offset, 9.0, 8.0),
                        point(scale, offset, 9.0, 12.6),
                    );
                    builder.bezier_curve_to(
                        point(scale, offset, 9.0, 17.0),
                        point(scale, offset, 12.4, 20.2),
                        point(scale, offset, 16.8, 20.5),
                    );
                    builder.bezier_curve_to(
                        point(scale, offset, 13.8, 22.2),
                        point(scale, offset, 9.7, 21.6),
                        point(scale, offset, 7.0, 18.9),
                    );
                    builder.bezier_curve_to(
                        point(scale, offset, 3.0, 14.9),
                        point(scale, offset, 3.0, 8.5),
                        point(scale, offset, 7.0, 4.7),
                    );
                    builder.bezier_curve_to(
                        point(scale, offset, 9.9, 2.0),
                        point(scale, offset, 14.2, 1.6),
                        point(scale, offset, 17.5, 3.5),
                    );
                    builder.close();
                });
                self.stroke(&mut frame, &moon, scale);
            }
            Icon::Nodes => {
                self.circle(&mut frame, scale, offset, (6.0, 6.0), 2.0);
                self.circle(&mut frame, scale, offset, (18.0, 12.0), 2.0);
                self.circle(&mut frame, scale, offset, (6.0, 18.0), 2.0);
                self.line(&mut frame, scale, offset, (8.0, 6.8), (16.0, 11.2));
                self.line(&mut frame, scale, offset, (8.0, 17.2), (16.0, 12.8));
            }
            Icon::Restore => {
                let restore = Path::new(|builder| {
                    builder.move_to(point(scale, offset, 8.0, 5.0));
                    builder.line_to(point(scale, offset, 19.0, 5.0));
                    builder.line_to(point(scale, offset, 19.0, 16.0));
                    builder.move_to(point(scale, offset, 16.0, 8.0));
                    builder.line_to(point(scale, offset, 5.0, 8.0));
                    builder.line_to(point(scale, offset, 5.0, 19.0));
                    builder.line_to(point(scale, offset, 16.0, 19.0));
                    builder.close();
                });
                self.stroke(&mut frame, &restore, scale);
            }
            Icon::Search => {
                self.circle(&mut frame, scale, offset, (10.5, 10.5), 6.5);
                self.line(&mut frame, scale, offset, (15.5, 15.5), (21.0, 21.0));
            }
            Icon::Settings => {
                self.circle(&mut frame, scale, offset, (12.0, 12.0), 7.0);
                self.circle(&mut frame, scale, offset, (12.0, 12.0), 2.6);
                for (from, to) in [
                    ((12.0, 2.0), (12.0, 5.0)),
                    ((12.0, 19.0), (12.0, 22.0)),
                    ((2.0, 12.0), (5.0, 12.0)),
                    ((19.0, 12.0), (22.0, 12.0)),
                    ((4.9, 4.9), (7.0, 7.0)),
                    ((17.0, 17.0), (19.1, 19.1)),
                    ((4.9, 19.1), (7.0, 17.0)),
                    ((17.0, 7.0), (19.1, 4.9)),
                ] {
                    self.line(&mut frame, scale, offset, from, to);
                }
            }
            Icon::Sidebar | Icon::Workspace => {
                let panel = Path::rounded_rectangle(
                    point(scale, offset, 3.0, 4.0),
                    Size::new(18.0 * scale, 16.0 * scale),
                    iced::border::Radius::from(2.0 * scale),
                );
                self.stroke(&mut frame, &panel, scale);
                self.line(&mut frame, scale, offset, (9.0, 4.0), (9.0, 20.0));
                if self.icon == Icon::Workspace {
                    self.line(&mut frame, scale, offset, (9.0, 10.0), (21.0, 10.0));
                }
            }
        }

        vec![frame.into_geometry()]
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
        let scale = bounds.width.min(bounds.height) / 24.0;
        let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
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
            let distance = (index + 8 - self.phase % 8) % 8;
            let alpha = 1.0 - f32::from(distance) * 0.105;
            frame.stroke(
                &Path::line(from, to),
                Stroke::default()
                    .with_color(Color {
                        a: self.color.a * alpha,
                        ..self.color
                    })
                    .with_width(2.2 * scale),
            );
        }
        vec![frame.into_geometry()]
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
