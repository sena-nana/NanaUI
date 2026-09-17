use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

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

/// Fits a tool window into one available display, including partially offscreen frames.
pub fn fit_window_to_displays(
    position: (f64, f64),
    size: (f64, f64),
    displays: &[DisplayBounds],
) -> ((f64, f64), (f64, f64)) {
    let center = (position.0 + size.0 / 2.0, position.1 + size.1 / 2.0);
    let Some(display) = displays.iter().min_by(|a, b| {
        a.distance_squared(center, (0.0, 0.0))
            .total_cmp(&b.distance_squared(center, (0.0, 0.0)))
    }) else {
        return (position, size);
    };
    let size = (
        size.0.min(display.size.0).max(1.0),
        size.1.min(display.size.1).max(1.0),
    );
    (display.clamp_position(position, size), size)
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowEvent {
    /// Rejected before opening. DuplicateRequest does not finish the active
    /// request with that same identity.
    FileDialogRejected {
        id: WindowId,
        request_id: u64,
        error: nana_ui_core::FileDialogError,
    },
    /// Completion of one accepted window-owned dialog request.
    FileDialogCompleted {
        id: WindowId,
        result: nana_ui_core::FileDialogResult,
    },
    /// Auxiliary creation failed; no window is retained for this id.
    OpenFailed {
        id: WindowId,
        error: String,
    },
    /// Acknowledges the requested hit-testing change, including failures.
    MousePassthroughChanged {
        id: WindowId,
        enabled: bool,
        result: Result<(), String>,
    },
    /// Acknowledges a taskbar-entry request, including failures. `skip_taskbar`
    /// is the effective state: a failed request leaves the previous value.
    SkipTaskbarChanged {
        id: WindowId,
        skip_taskbar: bool,
        result: Result<(), String>,
    },
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
    /// A hovering mouse or pen pointer entered (`inside`) or left the client
    /// area. Delivered only on change. Touch contacts never report presence,
    /// and the leave a host-started native window drag produces is withheld
    /// until the pointer is reported again. Hiding the window reports a leave.
    PointerPresenceChanged {
        id: WindowId,
        inside: bool,
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
    /// The operating system switched between light and dark appearance.
    /// Only delivered on platforms that report it; see [`SystemAppearance`].
    AppearanceChanged {
        id: WindowId,
        appearance: SystemAppearance,
    },
    /// Effective fullscreen state, level and display. Delivered after `Ready`,
    /// after each fullscreen or level request, and when the platform reports a
    /// change — only when it differs from the previous delivery.
    ModeChanged {
        id: WindowId,
        mode: WindowModeState,
    },
}

/// Session identity of a connected display. Equal across enumerations within
/// one process; only macOS keeps it across reconnects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayId(pub u128);

/// One connected display as enumerated by the window host.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayInfo {
    pub id: DisplayId,
    pub name: Option<String>,
    pub physical_position: Option<(i32, i32)>,
    pub physical_size: Option<(u32, u32)>,
    pub scale_factor: f64,
    pub refresh_rate_millihertz: Option<std::num::NonZeroU32>,
    /// Never reported on Wayland.
    pub primary: bool,
}

impl DisplayInfo {
    /// Bounds in the global logical space of `WindowDescriptor::initial_position`,
    /// where `desktop_scale` converts desktop pixels to that space. The host
    /// uses one scale for every display wherever the desktop is a single pixel
    /// grid, so displays with different scale factors never overlap.
    pub fn logical_bounds(&self, desktop_scale: f64) -> Option<DisplayBounds> {
        let (x, y) = self.physical_position?;
        let (width, height) = self.physical_size?;
        let scale = desktop_scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        Some(DisplayBounds {
            position: (f64::from(x) / scale, f64::from(y) / scale),
            size: (f64::from(width) / scale, f64::from(height) / scale),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FullscreenMode {
    /// Borderless fullscreen at the display's current video mode.
    #[default]
    Borderless,
    /// macOS fullscreen without a separate Space; `Borderless` elsewhere.
    Simple,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FullscreenRequest {
    pub mode: FullscreenMode,
    /// `None` keeps the window's current display.
    pub display: Option<DisplayId>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum WindowLevel {
    #[default]
    Normal,
    AlwaysOnTop,
    AlwaysOnBottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowModeState {
    /// `Simple` is reported only on macOS.
    pub fullscreen: Option<FullscreenMode>,
    /// The level last applied by the host; platforms do not report level
    /// changes made outside the application.
    pub level: WindowLevel,
    pub display: Option<DisplayId>,
}

/// Operating-system light/dark preference.
///
/// Platforms that do not report a preference yield `None` from
/// `system_appearance` queries and never emit
/// [`WindowEvent::AppearanceChanged`]; consumers keep their stored choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemAppearance {
    Light,
    Dark,
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
pub struct WindowDescriptor {
    pub title: String,
    pub initial_size: (f64, f64),
    pub minimum_size: (f64, f64),
    pub initial_position: Option<(f64, f64)>,
    pub maximized: bool,
    pub transparent: bool,
    /// Initial visibility, applied only after document initialization succeeds.
    pub visible: bool,
    pub always_on_top: bool,
    /// Enter fullscreen when the window is first shown. If the display is no
    /// longer connected the window opens without fullscreen.
    pub fullscreen: Option<FullscreenRequest>,
    /// Whether initially showing this window may activate it.
    pub focus_on_show: bool,
    /// Keep the complete restored frame inside the nearest display work area.
    pub constrain_to_work_area: bool,
    /// Keep this window out of the taskbar. The outcome is reported through
    /// [`WindowEvent::SkipTaskbarChanged`]; platforms without a per-window
    /// taskbar entry report `Unsupported` instead of pretending.
    pub skip_taskbar: bool,
    /// Host-chosen identity for restoring this window's last frame. Window
    /// ids are not stable across process restarts; this key is.
    pub persist_key: Option<String>,
    /// Application-chosen window kind, opaque to the host. Read it back from
    /// `RuntimeProgramContext::window_tag` while building the document and in
    /// every later callback for this window, so service-allocated ids never
    /// have to be matched to requests by order.
    pub tag: Option<Arc<str>>,
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

impl Default for WindowDescriptor {
    fn default() -> Self {
        Self::new("NanaUI")
    }
}

impl WindowDescriptor {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            initial_size: (1200.0, 800.0),
            minimum_size: (760.0, 520.0),
            initial_position: None,
            maximized: false,
            transparent: false,
            visible: true,
            always_on_top: false,
            fullscreen: None,
            focus_on_show: true,
            constrain_to_work_area: false,
            skip_taskbar: false,
            persist_key: None,
            tag: None,
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

    /// Tag this window with an application-chosen kind.
    pub fn tag(mut self, tag: impl Into<Arc<str>>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Remember this window's last size, position and maximized state.
    pub fn persist_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.persist_key = (!key.is_empty()).then_some(key);
        self
    }
}

/// Host-owned mouse-passthrough policy. Widgets never see the native handle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MousePassthroughMode {
    /// Window receives pointer events normally.
    #[default]
    Off,
    /// Native hit-testing is off for the whole window. Overlay does not
    /// receive pointer events and cannot recover itself.
    Passthrough,
    /// Native hit-testing stays off while the pointer is over empty
    /// transparent client area. The host samples the global pointer and
    /// restores hit-testing over opaque or interactive content.
    Forward,
}

impl MousePassthroughMode {
    pub fn passthrough(enabled: bool) -> Self {
        if enabled {
            Self::Passthrough
        } else {
            Self::Off
        }
    }

    pub fn forward(enabled: bool) -> Self {
        if enabled { Self::Forward } else { Self::Off }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowCommand {
    /// Disable native pointer hit testing. Always emits MousePassthroughChanged.
    SetMousePassthrough {
        id: WindowId,
        enabled: bool,
    },
    /// Enable host-owned Forward passthrough. Always emits MousePassthroughChanged.
    SetMousePassthroughForward {
        id: WindowId,
        enabled: bool,
    },
    Open {
        id: WindowId,
        settings: WindowDescriptor,
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
    /// `None` leaves fullscreen. A display that is not connected leaves the
    /// window unchanged; `WindowHandle::set_fullscreen` reports that error.
    SetFullscreen {
        id: WindowId,
        fullscreen: Option<FullscreenRequest>,
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
    /// Show or hide the taskbar entry. Always emits SkipTaskbarChanged.
    SetSkipTaskbar {
        id: WindowId,
        skip_taskbar: bool,
    },
    /// Per-window icon. `None` reapplies the registered or default mark.
    SetIcon {
        id: WindowId,
        icon: Option<WindowIcon>,
    },
    /// Application menu bar. `None` removes it.
    ///
    /// The menu is described by the application but installed by the host,
    /// because on Windows it belongs to a window and controls never reach a
    /// window handle. Selections come back through
    /// `nana_window::take_menu_activations`.
    SetMenuBar {
        id: WindowId,
        bar: Option<nana_ui_core::MenuBar>,
    },
    /// Open the system file dialog owned by this window.
    ///
    /// The host opens it because the dialog needs the parent window handle,
    /// which controls never reach; `PathField` still only emits
    /// `BrowseRequested`. The outcome — including a cancel — arrives through
    /// [`WindowEvent::FileDialogCompleted`], waking the host immediately.
    OpenFileDialog {
        id: WindowId,
        request: nana_ui_core::FileDialogRequest,
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
        let settings = WindowDescriptor::new("Scene");
        assert!(!settings.system_caption);
        assert!(!settings.skip_taskbar);
        assert!(settings.icon.is_none());
        assert!(matches!(
            WindowCommand::Drag(WindowId::PRIMARY),
            WindowCommand::Drag(_)
        ));
        assert_eq!(MousePassthroughMode::default(), MousePassthroughMode::Off);
        assert_eq!(
            MousePassthroughMode::forward(true),
            MousePassthroughMode::Forward
        );
        assert_eq!(
            MousePassthroughMode::passthrough(false),
            MousePassthroughMode::Off
        );
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
    #[test]
    fn strict_restore_keeps_controls_reachable_and_resizes_oversized_frames() {
        let screens = [DisplayBounds {
            position: (0.0, 40.0),
            size: (1920.0, 1000.0),
        }];
        assert_eq!(
            fit_window_to_displays((1800.0, -100.0), (420.0, 640.0), &screens),
            ((1500.0, 40.0), (420.0, 640.0))
        );
        assert_eq!(
            fit_window_to_displays((5000.0, 2000.0), (3000.0, 2000.0), &screens),
            ((0.0, 40.0), (1920.0, 1000.0))
        );
        assert_eq!(
            fit_window_to_displays((500.0, 100.0), (420.0, 640.0), &screens),
            ((500.0, 100.0), (420.0, 640.0))
        );
    }
}
