use std::fmt;

use iced::widget::canvas;
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};
use qrcode::{Color as ModuleColor, QrCode};

const QUIET_ZONE_MODULES: usize = 4;

/// A scanner-safe QR code rendered through Iced's WGPU-backed canvas.
///
/// Encoding and module layout are native Rust. The widget deliberately uses a
/// high-contrast opaque palette instead of theme colors because pairing and
/// login codes must remain readable in both light and dark application themes.
#[derive(Clone, Debug, PartialEq)]
pub struct QrCodeCanvas {
    modules: Vec<bool>,
    width: usize,
    size: f32,
}

impl QrCodeCanvas {
    pub fn encode(value: impl AsRef<[u8]>) -> Result<Self, QrCodeError> {
        let code = QrCode::new(value.as_ref()).map_err(|error| QrCodeError(error.to_string()))?;
        let width = code.width();
        let modules = code
            .to_colors()
            .into_iter()
            .map(|color| color == ModuleColor::Dark)
            .collect();
        Ok(Self {
            modules,
            width,
            size: 224.0,
        })
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(64.0);
        self
    }

    pub fn module_width(&self) -> usize {
        self.width
    }

    pub fn modules(&self) -> &[bool] {
        &self.modules
    }

    pub fn view<Message: 'static>(self) -> Element<'static, Message> {
        let size = self.size;
        canvas(self)
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .into()
    }

    fn module_geometry(&self, bounds: Rectangle) -> (f32, Point) {
        let module_count = self.width + QUIET_ZONE_MODULES * 2;
        let module_size = (bounds.width.min(bounds.height) / module_count as f32)
            .floor()
            .max(1.0);
        let rendered_size = module_size * module_count as f32;
        let origin = Point::new(
            ((bounds.width - rendered_size) / 2.0).floor(),
            ((bounds.height - rendered_size) / 2.0).floor(),
        );
        (module_size, origin)
    }
}

impl<Message> canvas::Program<Message> for QrCodeCanvas {
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
        let (module_size, origin) = self.module_geometry(bounds);
        let full_width = module_size * (self.width + QUIET_ZONE_MODULES * 2) as f32;
        frame.fill_rectangle(origin, Size::new(full_width, full_width), Color::WHITE);
        for (index, dark) in self.modules.iter().copied().enumerate() {
            if !dark {
                continue;
            }
            let x = index % self.width;
            let y = index / self.width;
            frame.fill_rectangle(
                Point::new(
                    origin.x + (x + QUIET_ZONE_MODULES) as f32 * module_size,
                    origin.y + (y + QUIET_ZONE_MODULES) as f32 * module_size,
                ),
                Size::new(module_size, module_size),
                Color::BLACK,
            );
        }
        vec![frame.into_geometry()]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrCodeError(String);

impl fmt::Display for QrCodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for QrCodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_pairing_uri_into_square_module_matrix() {
        let code = QrCodeCanvas::encode(
            "lilia-remote://pair?v=1&ticket=abc&challenge=def&bridge=http%3A%2F%2F10.0.0.2%3A41478",
        )
        .unwrap();
        assert!(code.module_width() >= 21);
        assert_eq!(
            code.modules.len(),
            code.module_width() * code.module_width()
        );
        assert!(code.modules.iter().any(|module| *module));
        assert!(code.modules.iter().any(|module| !*module));
    }

    #[test]
    fn module_geometry_preserves_four_module_quiet_zone() {
        let code = QrCodeCanvas::encode("native pairing").unwrap().size(220.0);
        let (module_size, origin) =
            code.module_geometry(Rectangle::new(Point::ORIGIN, Size::new(220.0, 220.0)));
        assert!(module_size >= 1.0);
        assert!(origin.x >= 0.0);
        assert!(origin.y >= 0.0);
        assert!(module_size * QUIET_ZONE_MODULES as f32 >= 4.0);
    }
}
