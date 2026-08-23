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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowIcon {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
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
        })
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
}
