use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Stable application-owned window identity. Platform backends keep their
/// native/winit window IDs private and map them to this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

impl WindowId {
    pub const PRIMARY: Self = Self(0);
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WindowGeometry {
    pub physical_position: Option<(i32, i32)>,
    pub physical_size: (u32, u32),
    pub logical_position: Option<(f32, f32)>,
    pub logical_size: (f32, f32),
    pub scale_factor: f32,
    pub maximized: bool,
}

/// Logical bounds of one display in the global logical coordinate space.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DisplayBounds {
    pub position: (f64, f64),
    pub size: (f64, f64),
}

impl DisplayBounds {
    fn intersects(&self, position: (f64, f64), size: (f64, f64)) -> bool {
        position.0 < self.position.0 + self.size.0
            && position.0 + size.0 > self.position.0
            && position.1 < self.position.1 + self.size.1
            && position.1 + size.1 > self.position.1
    }

    fn distance_squared(&self, position: (f64, f64), size: (f64, f64)) -> f64 {
        let dx = (self.position.0 - (position.0 + size.0))
            .max(position.0 - (self.position.0 + self.size.0))
            .max(0.0);
        let dy = (self.position.1 - (position.1 + size.1))
            .max(position.1 - (self.position.1 + self.size.1))
            .max(0.0);
        dx * dx + dy * dy
    }

    fn clamp_position(&self, position: (f64, f64), size: (f64, f64)) -> (f64, f64) {
        let max_x = self.position.0 + (self.size.0 - size.0).max(0.0);
        let max_y = self.position.1 + (self.size.1 - size.1).max(0.0);
        (
            position.0.clamp(self.position.0, max_x),
            position.1.clamp(self.position.1, max_y),
        )
    }
}

/// Keeps a restored window position on-screen.
///
/// Positions persisted from a previous session can point at a display that has
/// since been disconnected. The position passes through unchanged while the
/// window frame overlaps any display; otherwise it is clamped fully into the
/// display nearest to the requested frame. An empty display list also passes
/// the position through.
pub fn clamp_position_to_displays(
    position: (f64, f64),
    size: (f64, f64),
    displays: &[DisplayBounds],
) -> (f64, f64) {
    if displays
        .iter()
        .any(|display| display.intersects(position, size))
    {
        return position;
    }
    let Some(nearest) = displays.iter().min_by(|a, b| {
        a.distance_squared(position, size)
            .total_cmp(&b.distance_squared(position, size))
    }) else {
        return position;
    };
    nearest.clamp_position(position, size)
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowEvent {
    Ready {
        id: WindowId,
        geometry: WindowGeometry,
    },
    Resized {
        id: WindowId,
        geometry: WindowGeometry,
    },
    Moved {
        id: WindowId,
        geometry: WindowGeometry,
    },
    VisibilityChanged {
        id: WindowId,
        hidden: bool,
    },
    FocusChanged {
        id: WindowId,
        focused: bool,
    },
    Ime {
        id: WindowId,
        event: crate::ImeEvent,
    },
    CloseRequested {
        id: WindowId,
    },
    Closed {
        id: WindowId,
    },
    FileHovered {
        id: WindowId,
        paths: Vec<PathBuf>,
        position: Option<(f32, f32)>,
    },
    FileDropped {
        id: WindowId,
        paths: Vec<PathBuf>,
        position: Option<(f32, f32)>,
    },
    FileHoverCancelled {
        id: WindowId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextInputRequest {
    pub enabled: bool,
    pub cursor_area: Option<nana_ui_core::LogicalRect>,
    pub purpose: TextInputPurpose,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextInputPurpose {
    #[default]
    Normal,
    Password,
    Terminal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowRole {
    #[default]
    Main,
    Tool,
}

/// Unpremultiplied 32-bit RGBA window / taskbar / Dock identity image.
///
/// By default the icon is treated as a full-bleed brand mark: when it is
/// applied as the macOS Dock icon it is fitted into the system icon grid
/// automatically. Artwork that already carries platform margins opts out via
/// [`WindowIcon::exact_pixels`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowIcon {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    exact_pixels: bool,
}

impl WindowIcon {
    pub fn from_rgba(rgba: Vec<u8>, width: u32, height: u32) -> Result<Self, WindowIconError> {
        if width == 0 || height == 0 {
            return Err(WindowIconError::Empty);
        }
        if !rgba.len().is_multiple_of(4) {
            return Err(WindowIconError::ByteCountNotDivisibleBy4 {
                byte_count: rgba.len(),
            });
        }
        let pixels = rgba.len() / 4;
        let expected = width as usize * height as usize;
        if pixels != expected {
            return Err(WindowIconError::DimensionsMismatch {
                width,
                height,
                pixel_count: pixels,
            });
        }
        Ok(Self {
            rgba,
            width,
            height,
            exact_pixels: false,
        })
    }

    /// Declare the artwork already follows the platform icon grid, so the
    /// Dock application must use the pixels as-is.
    pub fn exact_pixels(mut self, exact: bool) -> Self {
        self.exact_pixels = exact;
        self
    }

    pub fn uses_exact_pixels(&self) -> bool {
        self.exact_pixels
    }

    pub fn set_exact_pixels(&mut self, exact: bool) {
        self.exact_pixels = exact;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowIconError {
    Empty,
    ByteCountNotDivisibleBy4 {
        byte_count: usize,
    },
    DimensionsMismatch {
        width: u32,
        height: u32,
        pixel_count: usize,
    },
}

impl std::fmt::Display for WindowIconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "window icon width and height must be non-zero"),
            Self::ByteCountNotDivisibleBy4 { byte_count } => write!(
                f,
                "window icon byte count {byte_count} is not divisible by 4"
            ),
            Self::DimensionsMismatch {
                width,
                height,
                pixel_count,
            } => write!(
                f,
                "window icon {width}x{height} expects {} pixels, got {pixel_count}",
                (*width as usize) * (*height as usize)
            ),
        }
    }
}

impl std::error::Error for WindowIconError {}

fn registered_icon_slot() -> &'static Mutex<Option<WindowIcon>> {
    static SLOT: OnceLock<Mutex<Option<WindowIcon>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn registered_icon_guard() -> std::sync::MutexGuard<'static, Option<WindowIcon>> {
    registered_icon_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Process-wide application identity. Used when a window does not set its own icon.
pub fn register_application_icon(icon: WindowIcon) {
    *registered_icon_guard() = Some(icon);
}

/// Forget a previously registered application icon.
pub fn clear_registered_application_icon() {
    *registered_icon_guard() = None;
}

fn registered_application_icon() -> Option<WindowIcon> {
    registered_icon_guard().clone()
}

/// Per-window override, else the process-wide registered icon.
pub fn resolve_window_icon(per_window: Option<&WindowIcon>) -> Option<WindowIcon> {
    per_window.cloned().or_else(registered_application_icon)
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowSettings {
    pub title: String,
    pub initial_size: (f64, f64),
    pub minimum_size: (f64, f64),
    pub initial_position: Option<(f64, f64)>,
    pub maximized: bool,
    pub transparent: bool,
    pub always_on_top: bool,
    pub resizable: bool,
    pub role: WindowRole,
    pub modal: bool,
    pub parent: Option<WindowId>,
    /// When true, keep the platform caption instead of NanaUI client chrome.
    ///
    /// Product windows leave this false: macOS overlays a transparent titlebar
    /// and traffic lights; Windows/Linux are undecorated. Hosted examples
    /// without a custom title bar should set this so Windows still has a close
    /// button.
    pub system_caption: bool,
    /// Per-window override of the process application icon.
    pub icon: Option<WindowIcon>,
}

impl WindowSettings {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            initial_size: (1200.0, 800.0),
            minimum_size: (760.0, 520.0),
            initial_position: None,
            maximized: false,
            transparent: false,
            always_on_top: false,
            resizable: true,
            role: WindowRole::Main,
            modal: false,
            parent: None,
            system_caption: false,
            icon: None,
        }
    }

    pub fn system_caption(mut self, enabled: bool) -> Self {
        self.system_caption = enabled;
        self
    }

    pub fn initial_size(mut self, width: f64, height: f64) -> Self {
        self.initial_size = (width, height);
        self
    }

    pub fn minimum_size(mut self, width: f64, height: f64) -> Self {
        self.minimum_size = (width, height);
        self
    }

    pub fn icon(mut self, icon: WindowIcon) -> Self {
        self.icon = Some(icon);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowCommand {
    Open {
        id: WindowId,
        settings: WindowSettings,
    },
    Close(WindowId),
    Move {
        id: WindowId,
        position: (f32, f32),
    },
    SetTitle {
        id: WindowId,
        title: String,
    },
    SetBounds {
        id: WindowId,
        position: (f32, f32),
        size: (f32, f32),
    },
    SetFullscreen {
        id: WindowId,
        fullscreen: bool,
    },
    /// macOS 走 winit simple fullscreen(不切换 Space);其他平台回落原生 Borderless 全屏。
    SetSimpleFullscreen {
        id: WindowId,
        fullscreen: bool,
    },
    SetMinimized {
        id: WindowId,
        minimized: bool,
    },
    SetMaximized {
        id: WindowId,
        maximized: bool,
    },
    SetAlwaysOnTop {
        id: WindowId,
        always_on_top: bool,
    },
    /// Per-window icon. `None` reapplies the registered or default mark.
    SetIcon {
        id: WindowId,
        icon: Option<WindowIcon>,
    },
    /// Process-wide application icon. `None` clears a registration so the default mark is used.
    SetApplicationIcon {
        icon: Option<WindowIcon>,
    },
    Focus(WindowId),
    /// Start a native window move from the current pointer gesture.
    Drag(WindowId),
}

/// Client-area edge used to start a window resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowResizeEdge {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

/// Hit-test the outer frame of a client-area window. Corners win over edges.
pub fn window_resize_edge(
    logical_size: (f32, f32),
    x: f32,
    y: f32,
    thickness: f32,
) -> Option<WindowResizeEdge> {
    if !thickness.is_finite() || thickness <= 0.0 {
        return None;
    }
    let (width, height) = logical_size;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let left = x < thickness;
    let right = x >= width - thickness;
    let top = y < thickness;
    let bottom = y >= height - thickness;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(WindowResizeEdge::NorthWest),
        (_, true, true, _) => Some(WindowResizeEdge::NorthEast),
        (true, _, _, true) => Some(WindowResizeEdge::SouthWest),
        (_, true, _, true) => Some(WindowResizeEdge::SouthEast),
        (true, _, _, _) => Some(WindowResizeEdge::West),
        (_, true, _, _) => Some(WindowResizeEdge::East),
        (_, _, true, _) => Some(WindowResizeEdge::North),
        (_, _, _, true) => Some(WindowResizeEdge::South),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_chrome_is_the_default_window_contract() {
        let settings = WindowSettings::new("Scene");
        assert!(!settings.system_caption);
        assert!(settings.icon.is_none());
        assert!(matches!(
            WindowCommand::Drag(WindowId::PRIMARY),
            WindowCommand::Drag(_)
        ));
    }

    #[test]
    fn window_resize_edge_prefers_corners_and_ignores_the_interior() {
        let size = (200.0, 100.0);
        let hit = [
            (2.0, 50.0, Some(WindowResizeEdge::West)),
            (198.0, 50.0, Some(WindowResizeEdge::East)),
            (100.0, 2.0, Some(WindowResizeEdge::North)),
            (100.0, 98.0, Some(WindowResizeEdge::South)),
            (2.0, 2.0, Some(WindowResizeEdge::NorthWest)),
            (198.0, 2.0, Some(WindowResizeEdge::NorthEast)),
            (2.0, 98.0, Some(WindowResizeEdge::SouthWest)),
            (198.0, 98.0, Some(WindowResizeEdge::SouthEast)),
            (100.0, 50.0, None),
            (8.0, 50.0, None),
        ];
        for (x, y, expected) in hit {
            assert_eq!(window_resize_edge(size, x, y, 8.0), expected);
        }
        assert_eq!(window_resize_edge(size, 100.0, 50.0, 0.0), None);
        assert_eq!(window_resize_edge((0.0, 100.0), 0.0, 0.0, 8.0), None);
    }

    #[test]
    fn window_icon_rejects_mismatched_rgba() {
        assert!(WindowIcon::from_rgba(vec![1, 2, 3], 1, 1).is_err());
        assert!(WindowIcon::from_rgba(vec![1, 2, 3, 4], 0, 1).is_err());
        assert_eq!(
            WindowIcon::from_rgba(vec![1, 2, 3, 4], 1, 1).unwrap().width,
            1
        );
    }

    #[test]
    fn registered_icon_is_used_when_window_does_not_override() {
        clear_registered_application_icon();
        let icon = WindowIcon::from_rgba(vec![10, 20, 30, 40], 1, 1).unwrap();
        register_application_icon(icon.clone());
        assert_eq!(resolve_window_icon(None), Some(icon.clone()));
        let overlay = WindowIcon::from_rgba(vec![9, 8, 7, 6], 1, 1).unwrap();
        assert_eq!(resolve_window_icon(Some(&overlay)), Some(overlay));
        clear_registered_application_icon();
        assert!(resolve_window_icon(None).is_none());
    }

    #[test]
    fn window_identity_is_backend_neutral_and_stable() {
        assert_eq!(WindowId::PRIMARY.0, 0);
        assert!(WindowId(2) > WindowId(1));
    }

    #[test]
    fn lifecycle_and_text_input_contracts_are_backend_neutral() {
        let geometry = WindowGeometry {
            logical_size: (1280.0, 720.0),
            physical_size: (2560, 1440),
            scale_factor: 2.0,
            ..WindowGeometry::default()
        };
        assert!(matches!(
            WindowEvent::Ready {
                id: WindowId::PRIMARY,
                geometry,
            },
            WindowEvent::Ready { geometry, .. } if geometry.scale_factor == 2.0
        ));
        assert_eq!(TextInputPurpose::default(), TextInputPurpose::Normal);
    }

    #[test]
    fn clamp_position_keeps_frames_that_overlap_any_display() {
        let main = [DisplayBounds {
            position: (0.0, 0.0),
            size: (1920.0, 1080.0),
        }];
        assert_eq!(
            clamp_position_to_displays((100.0, 80.0), (888.0, 586.0), &main),
            (100.0, 80.0)
        );
        assert_eq!(
            clamp_position_to_displays((1800.0, 900.0), (888.0, 586.0), &main),
            (1800.0, 900.0)
        );
    }

    #[test]
    fn clamp_position_pulls_frames_from_disconnected_displays_back_in() {
        let main = [DisplayBounds {
            position: (0.0, 0.0),
            size: (1920.0, 1080.0),
        }];
        let side = [
            main[0],
            DisplayBounds {
                position: (1920.0, 0.0),
                size: (1080.0, 1920.0),
            },
        ];
        assert_eq!(
            clamp_position_to_displays((2100.0, 40.0), (888.0, 586.0), &side),
            (2100.0, 40.0)
        );
        assert_eq!(
            clamp_position_to_displays((2100.0, 40.0), (888.0, 586.0), &main),
            (1032.0, 40.0)
        );
        assert_eq!(
            clamp_position_to_displays((-2000.0, -1000.0), (888.0, 586.0), &main),
            (0.0, 0.0)
        );
        assert_eq!(
            clamp_position_to_displays((5000.0, 100.0), (888.0, 586.0), &side),
            (2112.0, 100.0)
        );
    }

    #[test]
    fn clamp_position_anchors_oversized_frames_and_passthrough_empty_displays() {
        let main = [DisplayBounds {
            position: (0.0, 0.0),
            size: (1920.0, 1080.0),
        }];
        assert_eq!(
            clamp_position_to_displays((-3000.0, 2000.0), (3000.0, 2000.0), &main),
            (0.0, 0.0)
        );
        assert_eq!(
            clamp_position_to_displays((5000.0, 5000.0), (888.0, 586.0), &[]),
            (5000.0, 5000.0)
        );
    }
}
