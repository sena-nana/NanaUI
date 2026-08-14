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
        }
    }

    pub fn initial_size(mut self, width: f64, height: f64) -> Self {
        self.initial_size = (width, height);
        self
    }

    pub fn minimum_size(mut self, width: f64, height: f64) -> Self {
        self.minimum_size = (width, height);
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
    Focus(WindowId),
}

#[cfg(test)]
mod tests {
    use super::*;

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
