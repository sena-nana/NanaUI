//! Surface geometry: physical size, scale and the safe area.

/// The bands the system draws over, in physical px: status bar and display
/// cutout, home indicator or navigation bar, the soft keyboard, and the side
/// cutouts of a landscape phone.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SafeArea {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl SafeArea {
    pub fn scaled(self, factor: f32) -> Self {
        Self {
            top: self.top * factor,
            right: self.right * factor,
            bottom: self.bottom * factor,
            left: self.left * factor,
        }
    }
}

/// What the shell knows about its surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobileViewport {
    /// Drawable size in physical px.
    pub physical_size: (u32, u32),
    /// Physical px per logical px: UIKit `contentScaleFactor`, Android
    /// `densityDpi / 160`.
    pub scale: f32,
    pub safe_area: SafeArea,
}

impl MobileViewport {
    pub fn new(physical_size: (u32, u32), scale: f32, safe_area: SafeArea) -> Self {
        Self {
            physical_size: (physical_size.0.max(1), physical_size.1.max(1)),
            scale: scale.max(0.25),
            safe_area,
        }
    }

    pub fn logical_size(&self) -> (f32, f32) {
        (
            self.physical_size.0 as f32 / self.scale,
            self.physical_size.1 as f32 / self.scale,
        )
    }

    pub fn logical_point(&self, physical_x: f32, physical_y: f32) -> [f32; 2] {
        [physical_x / self.scale, physical_y / self.scale]
    }

    /// The safe area in logical px, as the root pads by it.
    pub fn logical_safe_area(&self) -> SafeArea {
        self.safe_area.scaled(1.0 / self.scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_degenerate_surface_still_lays_out() {
        let viewport = MobileViewport::new((0, 0), 0.0, SafeArea::default());
        assert_eq!(viewport.physical_size, (1, 1));
        assert_eq!(viewport.scale, 0.25);
    }

    #[test]
    fn a_3x_phone_maps_physical_to_logical() {
        let safe = SafeArea {
            top: 177.0,
            right: 0.0,
            bottom: 102.0,
            left: 0.0,
        };
        let viewport = MobileViewport::new((1179, 2556), 3.0, safe);
        assert_eq!(viewport.logical_size(), (393.0, 852.0));
        assert_eq!(viewport.logical_point(30.0, 90.0), [10.0, 30.0]);
        assert_eq!(viewport.logical_safe_area().top, 59.0);
        assert_eq!(viewport.logical_safe_area().bottom, 34.0);
    }
}
