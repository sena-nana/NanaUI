use iced::widget::canvas;
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Theme, mouse};

use crate::theme::ThemeTokens;

/// A compact, non-interactive time-series visualization rendered by Iced's
/// WGPU-backed canvas. Labels and business-specific semantics stay with the
/// consuming application.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeSeriesChart {
    values: Vec<f64>,
    tokens: ThemeTokens,
}

impl TimeSeriesChart {
    pub fn new(values: impl IntoIterator<Item = f64>, tokens: ThemeTokens) -> Self {
        Self {
            values: values
                .into_iter()
                .map(|value| {
                    if value.is_finite() {
                        value.max(0.0)
                    } else {
                        0.0
                    }
                })
                .collect(),
            tokens,
        }
    }

    pub fn view<Message: 'static>(self) -> Element<'static, Message> {
        canvas(self)
            .width(Length::Fill)
            .height(Length::Fixed(148.0))
            .into()
    }

    fn points(&self, bounds: Rectangle) -> Vec<Point> {
        const INSET_X: f32 = 8.0;
        const INSET_Y: f32 = 10.0;
        let width = (bounds.width - INSET_X * 2.0).max(1.0);
        let height = (bounds.height - INSET_Y * 2.0).max(1.0);
        let maximum = self.values.iter().copied().fold(0.0_f64, f64::max).max(1.0);
        let denominator = self.values.len().saturating_sub(1).max(1) as f32;
        self.values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let x = INSET_X + width * index as f32 / denominator;
                let normalized = (*value / maximum).clamp(0.0, 1.0) as f32;
                Point::new(x, INSET_Y + height * (1.0 - normalized))
            })
            .collect()
    }
}

impl<Message> canvas::Program<Message> for TimeSeriesChart {
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
        let colors = self.tokens.colors;
        for division in 0..=3 {
            let y = 10.0 + (bounds.height - 20.0).max(1.0) * division as f32 / 3.0;
            frame.stroke(
                &canvas::Path::line(Point::new(8.0, y), Point::new(bounds.width - 8.0, y)),
                canvas::Stroke::default()
                    .with_color(with_alpha(colors.border_soft, 0.55))
                    .with_width(1.0),
            );
        }

        let points = self.points(bounds);
        if points.is_empty() {
            return vec![frame.into_geometry()];
        }
        let baseline = (bounds.height - 10.0).max(10.0);
        let area = canvas::Path::new(|builder| {
            builder.move_to(Point::new(points[0].x, baseline));
            for point in &points {
                builder.line_to(*point);
            }
            builder.line_to(Point::new(points[points.len() - 1].x, baseline));
            builder.close();
        });
        frame.fill(&area, with_alpha(colors.accent, 0.16));

        if points.len() == 1 {
            frame.fill(&canvas::Path::circle(points[0], 2.5), colors.accent_strong);
        } else {
            let line = canvas::Path::new(|builder| {
                builder.move_to(points[0]);
                for point in points.iter().skip(1) {
                    builder.line_to(*point);
                }
            });
            frame.stroke(
                &line,
                canvas::Stroke::default()
                    .with_color(colors.accent_strong)
                    .with_width(2.0),
            );
        }
        vec![frame.into_geometry()]
    }
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ThemeMode, ThemeModeExt, UI_METRICS};

    #[test]
    fn chart_normalizes_non_finite_and_negative_values() {
        let chart = TimeSeriesChart::new(
            [f64::NAN, -2.0, 4.0, f64::INFINITY],
            ThemeTokens::new(ThemeMode::Dark.colors(), UI_METRICS),
        );
        assert_eq!(chart.values, vec![0.0, 0.0, 4.0, 0.0]);
    }

    #[test]
    fn points_span_the_series_and_scale_to_the_largest_value() {
        let chart = TimeSeriesChart::new(
            [0.0, 5.0, 10.0],
            ThemeTokens::new(ThemeMode::Dark.colors(), UI_METRICS),
        );
        let points = chart.points(Rectangle::new(Point::ORIGIN, iced::Size::new(108.0, 120.0)));
        assert_eq!(points.len(), 3);
        assert_eq!(points[0], Point::new(8.0, 110.0));
        assert_eq!(points[1], Point::new(54.0, 60.0));
        assert_eq!(points[2], Point::new(100.0, 10.0));
    }
}
