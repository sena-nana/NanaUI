use iced::Point;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResizeAxis {
    Horizontal,
    Vertical,
}

/// Converts pointer movement into an absolute resize value.
///
/// The first pointer position preserves the grab offset. Every later value is
/// derived from that same position, so clamping never accumulates drag error.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ResizeDrag {
    axis: ResizeAxis,
    start_position: Option<Point>,
    start_value: f32,
    units_per_pixel: f32,
}

impl ResizeDrag {
    pub(crate) fn new(axis: ResizeAxis, start_value: f32, units_per_pixel: f32) -> Self {
        Self {
            axis,
            start_position: None,
            start_value,
            units_per_pixel,
        }
    }

    pub(crate) fn value(&mut self, position: Point) -> Option<f32> {
        let Some(start_position) = self.start_position else {
            self.start_position = Some(position);
            return None;
        };
        let delta = match self.axis {
            ResizeAxis::Horizontal => position.x - start_position.x,
            ResizeAxis::Vertical => position.y - start_position.y,
        };
        Some(self.start_value + delta * self.units_per_pixel)
    }
}
