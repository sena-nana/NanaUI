use iced::widget::canvas::{Path, Stroke};
use iced::widget::{canvas, text};
use iced::{Color, Pixels, Point, Rectangle, Renderer, Size, Theme, alignment, mouse};

use crate::theme::{Colors, ui_font};

/// A small native canvas that demonstrates the node-editor surface boundary.
///
/// The demo deliberately owns only presentation data. A host can later replace
/// this program with a node graph backed by its document model without changing
/// the surrounding NanaUI workspace regions.
#[derive(Debug, Clone, Copy)]
pub struct NodeCanvas {
    pub node_count: u32,
    pub colors: Colors,
}

impl NodeCanvas {
    fn node_positions(&self, bounds: Rectangle) -> [Point; 3] {
        let node_width = 140.0;
        let left = 24.0;
        let available_gap = (bounds.width - (node_width * 3.0) - (left * 2.0)).max(0.0);
        let gap = available_gap / 2.0;
        let y = ((bounds.height - 76.0) / 2.0).max(32.0);

        [
            Point::new(left, y),
            Point::new(left + node_width + gap, y),
            Point::new(left + (node_width + gap) * 2.0, y),
        ]
    }

    fn draw_grid(&self, frame: &mut canvas::Frame<Renderer>) {
        let color = self.colors.border.scale_alpha(0.32);
        let stroke = Stroke::default().with_color(color).with_width(1.0);
        let step = 24.0;

        let mut x = 0.0;
        while x <= frame.width() {
            frame.stroke(
                &Path::line(Point::new(x, 0.0), Point::new(x, frame.height())),
                stroke,
            );
            x += step;
        }

        let mut y = 0.0;
        while y <= frame.height() {
            frame.stroke(
                &Path::line(Point::new(0.0, y), Point::new(frame.width(), y)),
                stroke,
            );
            y += step;
        }
    }

    fn draw_connection(&self, frame: &mut canvas::Frame<Renderer>, from: Point, to: Point) {
        let path = Path::new(|builder| {
            let midpoint = (from.x + to.x) / 2.0;
            builder.move_to(from);
            builder.bezier_curve_to(Point::new(midpoint, from.y), Point::new(midpoint, to.y), to);
        });
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(self.colors.accent.scale_alpha(0.8))
                .with_width(2.0),
        );
    }

    fn draw_node(
        &self,
        frame: &mut canvas::Frame<Renderer>,
        top_left: Point,
        title: &str,
        detail: &str,
        accent: Color,
    ) {
        let size = Size::new(140.0, 76.0);
        let node = Path::rounded_rectangle(top_left, size, iced::border::Radius::from(10.0));
        frame.fill(&node, self.colors.surface);
        frame.stroke(
            &node,
            Stroke::default()
                .with_color(self.colors.border_strong)
                .with_width(1.0),
        );

        let indicator = Path::circle(Point::new(top_left.x + 14.0, top_left.y + 18.0), 4.0);
        frame.fill(&indicator, accent);
        frame.fill_text(canvas::Text {
            content: title.to_owned(),
            position: Point::new(top_left.x + 26.0, top_left.y + 10.0),
            color: self.colors.text,
            size: Pixels::from(13),
            line_height: text::LineHeight::default(),
            font: ui_font(iced::font::Weight::Normal),
            align_x: text::Alignment::Left,
            align_y: alignment::Vertical::Top,
            shaping: text::Shaping::Basic,
            max_width: f32::INFINITY,
            ..canvas::Text::default()
        });
        frame.fill_text(canvas::Text {
            content: detail.to_owned(),
            position: Point::new(top_left.x + 14.0, top_left.y + 40.0),
            color: self.colors.muted,
            size: Pixels::from(11),
            line_height: text::LineHeight::default(),
            font: ui_font(iced::font::Weight::Normal),
            align_x: text::Alignment::Left,
            align_y: alignment::Vertical::Top,
            shaping: text::Shaping::Basic,
            max_width: f32::INFINITY,
            ..canvas::Text::default()
        });
    }
}

impl<Message> canvas::Program<Message> for NodeCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        self.draw_grid(&mut frame);

        let positions = self.node_positions(bounds);
        let centers = positions.map(|position| Point::new(position.x + 140.0, position.y + 38.0));
        self.draw_connection(
            &mut frame,
            Point::new(centers[0].x, centers[0].y),
            Point::new(centers[1].x - 140.0, centers[1].y),
        );
        self.draw_connection(
            &mut frame,
            Point::new(centers[1].x, centers[1].y),
            Point::new(centers[2].x - 140.0, centers[2].y),
        );

        self.draw_node(
            &mut frame,
            positions[0],
            "输入",
            "Texture",
            self.colors.accent,
        );
        self.draw_node(
            &mut frame,
            positions[1],
            "处理",
            "Color Grade",
            self.colors.warning,
        );
        self.draw_node(
            &mut frame,
            positions[2],
            "输出",
            "Preview",
            self.colors.success,
        );

        let extra = self.node_count.saturating_sub(3);
        if extra > 0 {
            frame.fill_text(canvas::Text {
                content: format!("+{} 个节点", extra),
                position: Point::new(positions[1].x, positions[1].y + 100.0),
                color: self.colors.accent,
                size: Pixels::from(11),
                line_height: text::LineHeight::default(),
                font: ui_font(iced::font::Weight::Normal),
                align_x: text::Alignment::Left,
                align_y: alignment::Vertical::Top,
                shaping: text::Shaping::Basic,
                max_width: f32::INFINITY,
                ..canvas::Text::default()
            });
        }

        vec![frame.into_geometry()]
    }
}

pub fn view<'a, Message: 'a>(node_count: u32, colors: Colors) -> iced::Element<'a, Message> {
    canvas(NodeCanvas { node_count, colors })
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
}
