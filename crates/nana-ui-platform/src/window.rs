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

/// The content zoom a window accepts; see [`WindowCommand::SetContentZoom`].
pub const CONTENT_ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.25..=5.0;

/// `zoom` clamped into [`CONTENT_ZOOM_RANGE`]; `1.0` when it is not a finite
/// positive number.
pub fn valid_content_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() && zoom > 0.0 {
        zoom.clamp(*CONTENT_ZOOM_RANGE.start(), *CONTENT_ZOOM_RANGE.end())
    } else {
        1.0
    }
}

impl WindowGeometry {
    /// The geometry the window's content lives in under content `zoom`: the
    /// same physical surface, a scale factor `zoom` times the display's, and
    /// a logical size `zoom` times smaller. Layout, painting, hit testing,
    /// accessibility bounds and the IME caret all use it, so every logical
    /// unit — fixed widths, borders, icons, text — grows by `zoom` together.
    ///
    /// The window itself does not change: physical size, desktop position
    /// and maximized state are the window's, and window sizes (minimum
    /// size, `SetBounds`, persisted geometry) stay in the display's logical
    /// units. `zoom` is passed through [`valid_content_zoom`].
    pub fn content(self, zoom: f32) -> Self {
        let zoom = valid_content_zoom(zoom);
        if zoom == 1.0 {
            return self;
        }
        Self {
            logical_size: (self.logical_size.0 / zoom, self.logical_size.1 / zoom),
            scale_factor: self.scale_factor * zoom,
            ..self
        }
    }
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

/// Index of the display holding `position`, else of the one whose edge is
/// nearest to it; the first such display on a tie.
pub fn display_index_at(position: (f64, f64), displays: &[DisplayBounds]) -> Option<usize> {
    displays
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.distance_squared(position, (0.0, 0.0))
                .total_cmp(&b.distance_squared(position, (0.0, 0.0)))
        })
        .map(|(index, _)| index)
}

/// Fits a tool window into one available display, including partially offscreen frames.
pub fn fit_window_to_displays(
    position: (f64, f64),
    size: (f64, f64),
    displays: &[DisplayBounds],
) -> ((f64, f64), (f64, f64)) {
    let center = (position.0 + size.0 / 2.0, position.1 + size.1 / 2.0);
    let Some(display) = display_index_at(center, displays).map(|index| &displays[index]) else {
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
    /// The system "reduce motion" preference changed. Sent to every window;
    /// read the current value from `RuntimeProgramContext::reduced_motion`.
    ReducedMotionChanged {
        id: WindowId,
        reduced: bool,
    },
    /// The system high-contrast flag changed. Sent to every window; read the
    /// current value from `RuntimeProgramContext::high_contrast`.
    HighContrastChanged {
        id: WindowId,
        high_contrast: bool,
    },
    /// The system "reduce transparency" preference changed (macOS). Sent to
    /// every window; read the current value from
    /// `RuntimeProgramContext::reduced_transparency`.
    ReducedTransparencyChanged {
        id: WindowId,
        reduced: bool,
    },
    /// The system text-size factor changed (Windows "Text size"). Sent to
    /// every window; read the current value from
    /// `RuntimeProgramContext::text_scale`. NanaUI does not apply it.
    TextScaleChanged {
        id: WindowId,
        scale: f32,
    },
    /// A hovering mouse or pen pointer entered (`inside`) or left the client
    /// area. Delivered only on change. Touch contacts never report presence,
    /// and the leave a host-started native window drag produces is withheld
    /// until the pointer is reported again. Hiding the window reports a leave.
    PointerPresenceChanged {
        id: WindowId,
        inside: bool,
    },
    CloseRequested {
        id: WindowId,
    },
    Closed {
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
    /// Fullscreen that stays among the other windows: on macOS without a
    /// separate Space; on Windows filling the work area, so the taskbar stays
    /// above and usable; `Borderless` elsewhere.
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowRole {
    #[default]
    Main,
    Tool,
    /// Glued directly beneath its `parent`: the same client area, moving,
    /// resizing, hiding and changing level with it, never taking input or
    /// activation, and kept drawing while the parent covers it. It is a
    /// separate top-level window with its own title, so screen-capture
    /// software can capture it on its own; one visual window becomes two
    /// native ones. Requires `parent`; the host fills in its geometry, keeps
    /// it out of the taskbar and gives it no shadow, persistence or
    /// accessibility tree. Platforms without one fail to open it.
    Underlay,
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
    /// Client width over client height the window keeps, or `None`. See
    /// [`WindowDescriptor::content_aspect_ratio`].
    pub content_aspect_ratio: Option<f32>,
    pub initial_position: Option<(f64, f64)>,
    pub maximized: bool,
    pub transparent: bool,
    /// Desktop shadow; does not add client padding or alter input regions.
    pub shadow: crate::WindowShadow,
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
    /// Stable application/profile/workspace scope, independent of transient window IDs.
    pub restoration_scope: nana_ui_core::RestorationPath,
    /// Application-chosen window kind, opaque to the host. Read it back from
    /// `RuntimeProgramContext::window_tag` while building the document and in
    /// every later callback for this window, so service-allocated ids never
    /// have to be matched to requests by order.
    pub tag: Option<Arc<str>>,
    pub resizable: bool,
    pub role: WindowRole,
    pub modal: bool,
    /// Owner of an auxiliary top-level window. Desktop hosts keep it above
    /// its owner without confining it to the owner's client area. Auxiliaries
    /// inherit a topmost owner's band; their own requested level is retained
    /// when the owner returns to normal. Underlay uses the opposite ordering.
    /// An initially hidden owner does not suppress a visible auxiliary.
    pub parent: Option<WindowId>,
    /// When true, keep the platform caption instead of NanaUI client chrome.
    ///
    /// Product windows leave this false: macOS overlays a transparent titlebar
    /// and traffic lights; Windows/Linux are undecorated. Hosted examples
    /// without a custom title bar should set this so Windows still has a close
    /// button.
    pub system_caption: bool,
    /// Move this window through NanaUI's pointer-driven frame move instead of
    /// entering the platform's nested native move loop. This keeps the host
    /// event loop running while the window follows the pointer, at the cost
    /// of platform-specific native drag affordances such as Aero Snap.
    pub host_managed_drag: bool,
    /// Per-window override of the process application icon.
    pub icon: Option<WindowIcon>,
    /// How this window's frames should reach the screen.
    pub surface: WindowSurfacePreference,
    /// Requested colour space and dynamic range policy for this window's surface.
    pub presentation_policy: SurfacePresentationPolicy,
}

/// Host-owned colour presentation request for a native surface.
///
/// The request is deliberately independent of a renderer or a device. The host
/// resolves it against the complete `SurfaceCapabilities` table at each surface
/// bind, so a window can move between displays without creating another device
/// or queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SurfacePresentationPolicy {
    /// Prefer the safe sRGB SDR profile, with deterministic capability fallbacks.
    #[default]
    Auto,
    /// Request sRGB SDR output.
    Sdr,
    /// Request Display-P3 SDR output, falling back through SDR profiles.
    WideGamut,
    /// Request extended-sRGB HDR output, falling back through wide-gamut and SDR.
    Hdr,
    /// Explicitly request BT.2100 PQ output. If the surface cannot present it,
    /// prefer extended-linear HDR, then encoded HDR, wide-gamut SDR and sRGB.
    /// Transparent windows use PQ only when a full-alpha format (normally
    /// fp16) is advertised; two-alpha-bit targets fall back deterministically.
    HdrPq,
    /// Explicitly request BT.2100 HLG output for a host whose output contract
    /// benefits from HLG. Uses the same deterministic fallback as [`Self::HdrPq`].
    HdrHlg,
}

/// How a window's frames should reach the screen.
///
/// This is a per-window choice on purpose. One window needing a platform
/// compositor visual — a transparent overlay, a window shadow companion — must
/// not move a Settings window, a dialog or a popup onto that path with it.
/// Whether the process *can* present that way at all is the separate,
/// process-wide GPU backend question, because the backend and device are shared
/// by every window. No platform type crosses this boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowSurfacePreference {
    /// The host picks. A window that asks for a transparent client takes the
    /// compositor path where the process has one; every other window stays on
    /// the platform's ordinary window surface.
    #[default]
    Auto,
    /// Always the platform's ordinary window surface.
    NativeWindow,
    /// A platform compositor visual where the process can present through one.
    /// A window that cannot get it is told so rather than failing to open.
    Composition,
    /// A platform compositor visual or nothing.
    ///
    /// A window that cannot get one fails to open instead of presenting
    /// through the ordinary window surface. For hosts whose content is only
    /// correct on the compositor path and would rather not run at all than run
    /// wrong — every other window should prefer [`Self::Composition`], which
    /// reports the fallback and keeps the application alive.
    RequireComposition,
}

impl WindowSurfacePreference {
    /// Whether a window that cannot reach the compositor must fail to open
    /// rather than fall back.
    pub const fn requires_composition(self) -> bool {
        matches!(self, Self::RequireComposition)
    }
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
            content_aspect_ratio: None,
            initial_position: None,
            maximized: false,
            transparent: false,
            shadow: crate::WindowShadow::Auto,
            visible: true,
            always_on_top: false,
            fullscreen: None,
            focus_on_show: true,
            constrain_to_work_area: false,
            skip_taskbar: false,
            persist_key: None,
            restoration_scope: nana_ui_core::RestorationPath::root(),
            tag: None,
            resizable: true,
            role: WindowRole::Main,
            modal: false,
            parent: None,
            system_caption: false,
            host_managed_drag: false,
            icon: None,
            surface: WindowSurfacePreference::Auto,
            presentation_policy: SurfacePresentationPolicy::Auto,
        }
    }

    /// Choose how this window's frames reach the screen.
    pub const fn surface(mut self, surface: WindowSurfacePreference) -> Self {
        self.surface = surface;
        self
    }

    /// Choose the colour space and dynamic range policy for this window's surface.
    pub const fn presentation_policy(mut self, policy: SurfacePresentationPolicy) -> Self {
        self.presentation_policy = policy;
        self
    }

    pub const fn shadow(mut self, shadow: crate::WindowShadow) -> Self {
        self.shadow = shadow;
        self
    }

    pub fn system_caption(mut self, enabled: bool) -> Self {
        self.system_caption = enabled;
        self
    }

    /// Keep window movement in NanaUI's event loop while the primary button
    /// drags the custom title bar.
    pub const fn host_managed_drag(mut self, enabled: bool) -> Self {
        self.host_managed_drag = enabled;
        self
    }

    pub fn initial_size(mut self, width: f64, height: f64) -> Self {
        self.initial_size = (width, height);
        self
    }

    /// Keep this window above normal windows when the host presents it.
    pub const fn always_on_top(mut self, enabled: bool) -> Self {
        self.always_on_top = enabled;
        self
    }

    /// Omit this window from the platform taskbar or dock when supported.
    pub const fn skip_taskbar(mut self, enabled: bool) -> Self {
        self.skip_taskbar = enabled;
        self
    }

    /// Mark utility windows (such as a picture-in-picture player) as tools.
    pub const fn role(mut self, role: WindowRole) -> Self {
        self.role = role;
        self
    }

    pub fn minimum_size(mut self, width: f64, height: f64) -> Self {
        self.minimum_size = (width, height);
        self
    }

    /// Keep the client area at `ratio` (width over height), for a window
    /// whose content is one picture, such as a video player.
    ///
    /// The window opens at [`aspect_window_size`] of its initial (or
    /// restored) size, and its minimum becomes [`aspect_minimum_size`]: the
    /// smallest size with this ratio that breaks neither minimum edge, so give
    /// a minimum both orientations can live with (a square works for content
    /// that may be portrait). While the user drags a frame edge the opposite
    /// dimension follows. Maximized and fullscreen windows take the platform's
    /// size. Change it later with `WindowCommand::SetContentAspectRatio`.
    /// A ratio that is not finite and positive counts as `None`.
    ///
    /// Held during user resizes on Windows; elsewhere only the open size,
    /// conforming and the minimum apply.
    pub fn content_aspect_ratio(mut self, ratio: Option<f32>) -> Self {
        self.content_aspect_ratio = valid_aspect_ratio(ratio);
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

// Commands are issued a handful of times per window lifetime, never per frame;
// boxing `Open`'s descriptor would only move the size into every caller.
#[allow(clippy::large_enum_variant)]
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
    /// Lock the client area to `ratio` (width over height) or, with `None`,
    /// release it; see [`WindowDescriptor::content_aspect_ratio`]. A windowed
    /// window is resized to [`aspect_window_size`] of its current size at
    /// once; a maximized or fullscreen one when it returns to windowed.
    SetContentAspectRatio {
        id: WindowId,
        ratio: Option<f32>,
    },
    /// Fade the platform's native window buttons (macOS traffic lights) over
    /// `duration`; hidden buttons take no pointer input. The host keeps the
    /// state across native style changes such as leaving fullscreen.
    /// Platforms whose buttons are `AppTitleBar` controls ignore it: the
    /// application presents those with the title bar.
    SetNativeWindowControlsVisible {
        id: WindowId,
        visible: bool,
        duration: std::time::Duration,
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
    /// Change the desktop shadow of an open window. The applied outcome is
    /// reported through the window's presentation, not assumed.
    SetShadow {
        id: WindowId,
        shadow: crate::WindowShadow,
    },
    /// Zoom everything the window's UI draws by `zoom` (1.0 is none),
    /// composed with the display scale factor: see
    /// [`WindowGeometry::content`]. The window keeps its size and is not
    /// recreated; the next frame lays the document out in the smaller
    /// logical viewport and paints it at the larger scale. Clamped to
    /// [`CONTENT_ZOOM_RANGE`].
    SetContentZoom {
        id: WindowId,
        zoom: f32,
    },
    /// Change an open window's minimum and maximum client size, in the
    /// display's logical units; `None` removes that limit (a minimum of
    /// `None` is the platform's own). A windowed window outside the new
    /// limits is resized into them at once ([`size_within_limits`]); a
    /// maximized or fullscreen one keeps the platform's size. A maximum
    /// below the minimum is raised to it. A size that is not finite and
    /// positive leaves the window's limits unchanged.
    SetSizeLimits {
        id: WindowId,
        minimum: Option<(f64, f64)>,
        maximum: Option<(f64, f64)>,
    },
}

/// `size` brought inside `minimum` and `maximum`, each edge on its own. A
/// maximum below the minimum is raised to it: the minimum wins, as it does on
/// every platform.
pub fn size_within_limits(
    size: (f64, f64),
    minimum: Option<(f64, f64)>,
    maximum: Option<(f64, f64)>,
) -> (f64, f64) {
    let minimum = minimum.unwrap_or((0.0, 0.0));
    let maximum = maximum.map(|maximum| (maximum.0.max(minimum.0), maximum.1.max(minimum.1)));
    let edge = |value: f64, low: f64, high: Option<f64>| {
        let value = value.max(low);
        high.map_or(value, |high| value.min(high))
    };
    (
        edge(size.0, minimum.0, maximum.map(|maximum| maximum.0)),
        edge(size.1, minimum.1, maximum.map(|maximum| maximum.1)),
    )
}

/// `ratio` when it is finite and positive.
pub fn valid_aspect_ratio(ratio: Option<f32>) -> Option<f32> {
    ratio.filter(|ratio| ratio.is_finite() && *ratio > 0.0)
}

/// The minimum client size of a window locked to `ratio`: the smallest size
/// with that ratio whose edges are both at least `minimum`'s. One edge keeps
/// its minimum and the other grows, so a landscape minimum forces portrait
/// content tall; give a minimum both orientations can live with.
pub fn aspect_minimum_size(minimum: (f64, f64), ratio: f32) -> (f64, f64) {
    let ratio = f64::from(ratio);
    let width = minimum.0.max(minimum.1 * ratio);
    (width, width / ratio)
}

/// The client size a window locked to `ratio` takes from `size`.
///
/// It keeps the area of `size`, so switching between ratios and back
/// returns the same size, then scales down to fit inside `limit` (a work
/// area, when known) and up to [`aspect_minimum_size`] of `minimum`; the
/// minimum wins over the limit, as the platform's minimum would.
pub fn aspect_window_size(
    size: (f64, f64),
    ratio: f32,
    minimum: (f64, f64),
    limit: Option<(f64, f64)>,
) -> (f64, f64) {
    let ratio = f64::from(ratio);
    let area = (size.0 * size.1).max(1.0);
    let mut width = (area * ratio).sqrt();
    if let Some(limit) = limit.filter(|limit| limit.0 > 0.0 && limit.1 > 0.0) {
        width = width.min(limit.0).min(limit.1 * ratio);
    }
    let width = width.max(aspect_minimum_size(minimum, ratio as f32).0);
    (width, width / ratio)
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
    fn a_size_is_brought_inside_its_limits_and_the_minimum_wins() {
        let min = Some((400.0, 300.0));
        let max = Some((1200.0, 900.0));
        assert_eq!(size_within_limits((800.0, 600.0), min, max), (800.0, 600.0));
        assert_eq!(
            size_within_limits((200.0, 1000.0), min, max),
            (400.0, 900.0)
        );
        assert_eq!(
            size_within_limits((2000.0, 100.0), min, None),
            (2000.0, 300.0)
        );
        assert_eq!(
            size_within_limits((2000.0, 100.0), None, max),
            (1200.0, 100.0)
        );
        // A maximum under the minimum is raised to it.
        assert_eq!(
            size_within_limits((100.0, 100.0), Some((500.0, 500.0)), Some((300.0, 800.0))),
            (500.0, 500.0)
        );
    }

    #[test]
    fn content_zoom_composes_with_the_display_scale_and_keeps_the_window() {
        let window = WindowGeometry {
            physical_position: Some((100, 50)),
            physical_size: (1600, 1200),
            logical_position: Some((50.0, 25.0)),
            logical_size: (800.0, 600.0),
            scale_factor: 2.0,
            maximized: true,
        };
        let content = window.content(1.25);
        assert_eq!(content.scale_factor, 2.5);
        assert_eq!(content.logical_size, (640.0, 480.0));
        // The surface and the window's place on the desktop are unchanged.
        assert_eq!(content.physical_size, window.physical_size);
        assert_eq!(content.physical_position, window.physical_position);
        assert_eq!(content.logical_position, window.logical_position);
        assert!(content.maximized);
        // Logical size times scale is still the surface.
        assert_eq!(content.logical_size.0 * content.scale_factor, 1600.0);
        assert_eq!(window.content(1.0), window);
        assert_eq!(window.content(f32::NAN), window);
        assert_eq!(window.content(100.0).scale_factor, 10.0);
        assert_eq!(valid_content_zoom(0.0), 1.0);
        assert_eq!(valid_content_zoom(0.1), 0.25);
    }

    #[test]
    fn client_chrome_is_the_default_window_contract() {
        let settings = WindowDescriptor::new("Scene");
        assert!(!settings.system_caption);
        assert!(!settings.host_managed_drag);
        assert!(
            WindowDescriptor::new("Tool")
                .host_managed_drag(true)
                .host_managed_drag
        );
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

    #[test]
    fn an_aspect_minimum_grows_one_edge_and_keeps_the_other() {
        assert_eq!(aspect_minimum_size((320.0, 320.0), 2.0), (640.0, 320.0));
        assert_eq!(aspect_minimum_size((320.0, 320.0), 0.5), (320.0, 640.0));
        // A landscape minimum makes portrait content tall.
        assert_eq!(aspect_minimum_size((640.0, 360.0), 0.5), (640.0, 1280.0));
    }

    #[test]
    fn an_aspect_size_keeps_the_area_so_switching_back_returns_the_same_size() {
        let wide = aspect_window_size((960.0, 540.0), 16.0 / 9.0, (320.0, 320.0), None);
        assert!((wide.0 - 960.0).abs() < 1e-3 && (wide.1 - 540.0).abs() < 1e-3);
        let tall = aspect_window_size(wide, 9.0 / 16.0, (320.0, 320.0), None);
        assert!((tall.0 - 540.0).abs() < 1e-3 && (tall.1 - 960.0).abs() < 1e-3);
        let back = aspect_window_size(tall, 16.0 / 9.0, (320.0, 320.0), None);
        assert!((back.0 - 960.0).abs() < 1e-3 && (back.1 - 540.0).abs() < 1e-3);
    }

    #[test]
    fn an_aspect_size_fits_the_limit_but_not_below_the_minimum() {
        // A portrait video on a 1366 x 728 work area fits its height.
        let fitted = aspect_window_size((960.0, 540.0), 0.5, (200.0, 200.0), Some((1366.0, 728.0)));
        assert_eq!(fitted, (364.0, 728.0));
        // The minimum wins over a work area too small for it.
        let floor = aspect_window_size((960.0, 540.0), 1.0, (500.0, 500.0), Some((400.0, 400.0)));
        assert_eq!(floor, (500.0, 500.0));
    }

    #[test]
    fn an_invalid_aspect_ratio_is_no_lock() {
        let descriptor = WindowDescriptor::new("Player").content_aspect_ratio(Some(f32::NAN));
        assert_eq!(descriptor.content_aspect_ratio, None);
        let descriptor = descriptor.content_aspect_ratio(Some(0.0));
        assert_eq!(descriptor.content_aspect_ratio, None);
        let descriptor = descriptor.content_aspect_ratio(Some(1.5));
        assert_eq!(descriptor.content_aspect_ratio, Some(1.5));
    }

    #[test]
    fn window_descriptor_presentation_policy_defaults_to_auto_and_sets_policy() {
        let descriptor = WindowDescriptor::new("Color");
        assert_eq!(
            descriptor.presentation_policy,
            SurfacePresentationPolicy::Auto
        );
        let hdr = descriptor.presentation_policy(SurfacePresentationPolicy::Hdr);
        assert_eq!(hdr.presentation_policy, SurfacePresentationPolicy::Hdr);
    }
}
