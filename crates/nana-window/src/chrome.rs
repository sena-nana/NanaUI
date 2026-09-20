use raw_window_handle::HasWindowHandle;

/// Win32 `WS_CAPTION` (`WS_BORDER | WS_DLGFRAME`).
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
const WS_CAPTION: isize = 0x00C0_0000;
/// Win32 `WS_THICKFRAME`, spelled `WS_SIZEBOX` by winit.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
const WS_THICKFRAME: isize = 0x0004_0000;
/// Win32 `WS_SYSMENU`.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
const WS_SYSMENU: isize = 0x0008_0000;

/// Style bits a transparent client-chrome window must not carry.
///
/// DWM renders a caption and a drop shadow for any HWND it considers framed,
/// underneath the client area. winit's undecorated window keeps every one of
/// these bits and only extends the client over them through `WM_NCCALCSIZE`,
/// so an opaque client hides that rendering and a transparent one shows it
/// through. `WS_SYSMENU` is in the mask because it is what puts buttons in the
/// caption DWM draws; it is the first bit to take back out if the taskbar's
/// own minimize or Aero Peek turn out to need it.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
const FRAMELESS_STYLES: isize = WS_CAPTION | WS_THICKFRAME | WS_SYSMENU;

/// Prepares native titlebar dragging and client-chrome window shape for a
/// custom titlebar `titlebar_height` logical points tall.
///
/// `rounded_corners` follows the surface actually in use, not how the window
/// was created: an opaque client takes the Windows 11 round clip and the system
/// stroke paired with it, a transparent one neither, so DWM leaves no outline
/// around an HWND rectangle its surface no longer fills.
pub fn prepare_client_chrome<W: HasWindowHandle + ?Sized>(
    window: &W,
    titlebar_height: f64,
    rounded_corners: bool,
) -> bool {
    let prepared = prepare_custom_title_bar(window);
    let prepared = center_traffic_lights(window, titlebar_height) && prepared;
    #[cfg(target_os = "windows")]
    let prepared = apply_window_shape(window, rounded_corners) && prepared;
    #[cfg(not(target_os = "windows"))]
    let _ = rounded_corners;
    prepared
}

/// How a transparent client stops DWM rendering a non-client area underneath it.
///
/// winit's undecorated window keeps `WS_CAPTION | WS_THICKFRAME | WS_SYSMENU`
/// and only extends the client over them through `WM_NCCALCSIZE`, so DWM goes
/// on drawing a caption, its buttons and a drop shadow *below* the client. An
/// opaque client hides that; a transparent one shows it through.
///
/// There are two ways out, and they cost different things:
///
/// - [`Self::StripFrameStyles`] takes the bits away. It is measured to work,
///   and it gives up the system behaviour those bits carry: Aero Snap, the
///   Windows 11 Snap Layouts hover menu on the maximize button, the Alt+Space
///   system menu, and the system minimize/maximize animations.
/// - [`Self::SuppressNonClientRendering`] keeps the bits and asks DWM not to
///   render the non-client area (`DWMWA_NCRENDERING_POLICY = DWMNCRP_DISABLED`).
///   If that holds, a transparent window keeps every one of those system
///   behaviours.
///
/// Which one is correct is a measurement, not a deduction: `DwmSetWindowAttribute`
/// reports success whether or not the policy has the intended effect, and the
/// first acceptance round tested it on a window that was *not* presenting
/// through DirectComposition. So this is a switch with a recorded default
/// rather than a branch that detects its own failure — see
/// `docs/window.md` for the comparison the default comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NonClientRenderingStrategy {
    /// Remove the frame style bits. The measured default.
    #[default]
    StripFrameStyles,
    /// Keep them and disable DWM's non-client rendering instead.
    SuppressNonClientRendering,
}

impl NonClientRenderingStrategy {
    /// The strategy this process uses.
    ///
    /// `NANA_WINDOWS_NC_STRATEGY=suppress` selects
    /// [`Self::SuppressNonClientRendering`], which is how the acceptance probe
    /// runs the two side by side on one machine. Anything else, including
    /// unset, is the default.
    pub fn from_env() -> Self {
        match std::env::var("NANA_WINDOWS_NC_STRATEGY").as_deref() {
            Ok("suppress") => Self::SuppressNonClientRendering,
            _ => Self::StripFrameStyles,
        }
    }

    /// Whether a transparent client on this strategy loses its frame styles,
    /// and with them Aero Snap, Snap Layouts, Alt+Space and the system
    /// minimize/maximize animations.
    pub const fn strips_frame_styles(self) -> bool {
        matches!(self, Self::StripFrameStyles)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::StripFrameStyles => "剥离 frame 样式位",
            Self::SuppressNonClientRendering => "保留 frame 样式位并关闭非客户区渲染",
        }
    }
}

/// Asks DWM whether to render this window's non-client area.
///
/// Keeping the frame styles and turning this off is the path that would let a
/// transparent window keep Aero Snap, Snap Layouts, Alt+Space and the system
/// window animations. Both directions are written, because a window whose
/// material flips at runtime would otherwise keep whichever policy it was last
/// given. Returns whether DWM accepted the attribute — which is not the same as
/// the policy having the intended visual effect, so a caller must not read a
/// `true` here as "the caption is gone". Other platforms no-op.
pub fn set_non_client_rendering<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> bool {
    #[cfg(target_os = "windows")]
    {
        set_nc_rendering_policy(window, enabled)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, enabled);
        true
    }
}

/// Sets the Win32 frame styles on a client-chrome window.
///
/// An opaque frameless window keeps them and extends the client through
/// `WM_NCCALCSIZE`, so DWM still rounds it and hangs a shadow on it. A
/// transparent one drops them: DWM must not render a frame an HWND's own
/// surface would then show through. A window whose material flips at runtime
/// needs both directions. Other platforms no-op.
///
/// `resizable` only reaches the restoring direction. winit derives
/// `WS_THICKFRAME` from its own resizable flag, so handing it back to a window
/// that never had it would grow a system resize border.
pub fn set_frameless_styles<W: HasWindowHandle + ?Sized>(
    window: &W,
    frameless: bool,
    resizable: bool,
) -> bool {
    #[cfg(target_os = "windows")]
    {
        set_frameless_style(window, frameless, resizable)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, frameless, resizable);
        true
    }
}

/// Says what the frame styles should be without writing them.
///
/// Sends no window message, so a host still inside `can_create_surfaces` can
/// call it: the strip lands on whatever winit writes next, and showing the
/// window is itself such a write — `apply_diff` rewrites the whole style and
/// sends its own `SetWindowPos(SWP_FRAMECHANGED)` straight after. So the window
/// is never presented with a frame, without the host sending a frame change of
/// its own from inside a surface callback. Other platforms no-op.
pub fn arm_frameless_guard<W: HasWindowHandle + ?Sized>(window: &W, frameless: bool) -> bool {
    #[cfg(target_os = "windows")]
    {
        let Some(hwnd) = chrome_hwnd(window) else {
            return false;
        };
        install_style_guard(hwnd, frameless_mask(frameless))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, frameless);
        true
    }
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
const fn client_chrome_style(current: isize, frameless: bool, resizable: bool) -> isize {
    if frameless {
        current & !FRAMELESS_STYLES
    } else if resizable {
        current | FRAMELESS_STYLES
    } else {
        current | WS_CAPTION | WS_SYSMENU
    }
}

/// The bits a `WM_STYLECHANGING` guard leaves out of the style being written.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
const fn guarded_style(style_new: isize, mask: isize) -> isize {
    style_new & !mask
}

/// Prepares native titlebar dragging for NanaUI's custom titlebar regions.
pub fn prepare_custom_title_bar<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    set_drag_enabled(window, false)
}

/// Performs one explicit NanaUI titlebar drag.
///
/// Returns `true` only when a native drag started. The platform then owns the
/// button release, so the caller must end its pointer gesture itself.
pub fn drag_custom_title_bar<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    drag(window)
}

/// Client-chrome edge used to start a native frame resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameResizeEdge {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

/// Starts an OS-owned frame resize. Windows uses `WM_NCLBUTTONDOWN`; other
/// platforms return false so the host can fall back.
pub fn resize_custom_frame<W: HasWindowHandle + ?Sized>(window: &W, edge: FrameResizeEdge) -> bool {
    #[cfg(target_os = "windows")]
    {
        resize_windows(window, edge)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, edge);
        false
    }
}

/// Whether the OS owns an active frame-resize gesture for this window
/// (AppKit's native live-resize tracking loop). The Win32 size-move hook is
/// host-side state, so other platforms never report one.
pub fn native_live_resize_active<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    #[cfg(target_os = "macos")]
    {
        appkit_window(window).is_some_and(|window| window.inLiveResize())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        false
    }
}

/// Pins or unpins the window's Metal layer to present inside Core Animation
/// transactions. While a resize gesture moves the window frame, the
/// compositor otherwise scales the previous drawable to the new frame between
/// the frame change and the next present; a transaction present closes that
/// window. Returns whether a CAMetalLayer was found and updated.
pub fn set_present_transaction<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        set_metal_present_transaction(window, enabled)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, enabled);
        false
    }
}

#[cfg(target_os = "macos")]
fn set_metal_present_transaction<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> bool {
    use objc2_app_kit::NSView;
    use objc2_quartz_core::CAMetalLayer;
    use raw_window_handle::RawWindowHandle;

    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    // SAFETY: the AppKit handle's ns_view is a live NSView owned by this window.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(layer) = view.layer() else {
        return false;
    };
    let Ok(metal) = layer.downcast::<CAMetalLayer>() else {
        return false;
    };
    metal.setPresentsWithTransaction(enabled);
    true
}

/// Captures a window frame so later pointer moves can resize origin and size
/// without a nested OS size-move loop. The window's minimum track size is
/// queried once at `begin` and clamps every update; there is no max clamp.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug, Clone, Copy)]
pub struct LiveFrameResize {
    edge: FrameResizeEdge,
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    mouse_x: f64,
    mouse_y: f64,
    min: (f64, f64),
}

#[cfg(target_os = "macos")]
impl LiveFrameResize {
    pub fn begin<W: HasWindowHandle + ?Sized>(window: &W, edge: FrameResizeEdge) -> Option<Self> {
        let window = appkit_window(window)?;
        let frame = window.frame();
        let mouse = objc2_app_kit::NSEvent::mouseLocation();
        let min_size = window.minSize();
        let content_min = window.contentMinSize();
        Some(Self {
            edge,
            origin_x: frame.origin.x,
            origin_y: frame.origin.y,
            width: frame.size.width,
            height: frame.size.height,
            mouse_x: mouse.x,
            mouse_y: mouse.y,
            min: (
                min_size.width.max(content_min.width),
                min_size.height.max(content_min.height),
            ),
        })
    }

    pub fn update<W: HasWindowHandle + ?Sized>(&self, window: &W) -> bool {
        let Some(window) = appkit_window(window) else {
            return false;
        };
        let mouse = objc2_app_kit::NSEvent::mouseLocation();
        let next = live_frame_after_delta(
            [self.origin_x, self.origin_y, self.width, self.height],
            mouse.x - self.mouse_x,
            mouse.y - self.mouse_y,
            self.edge,
            self.min,
            false,
        );
        window.setFrame_display(
            objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(next[0], next[1]),
                objc2_foundation::NSSize::new(next[2], next[3]),
            ),
            true,
        );
        true
    }

    pub fn end<W: HasWindowHandle + ?Sized>(self, _window: &W) {}
}

#[cfg(target_os = "windows")]
impl LiveFrameResize {
    pub fn begin<W: HasWindowHandle + ?Sized>(window: &W, edge: FrameResizeEdge) -> Option<Self> {
        let hwnd = win32_hwnd(window)?;
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        let mut mouse = windows_sys::Win32::Foundation::POINT::default();
        unsafe {
            if windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) == 0 {
                return None;
            }
            if windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut mouse) == 0 {
                return None;
            }
            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
        }
        let min = win32_min_track_size(hwnd);
        Some(Self {
            edge,
            origin_x: f64::from(rect.left),
            origin_y: f64::from(rect.top),
            width: f64::from(rect.right - rect.left),
            height: f64::from(rect.bottom - rect.top),
            mouse_x: f64::from(mouse.x),
            mouse_y: f64::from(mouse.y),
            min,
        })
    }

    pub fn update<W: HasWindowHandle + ?Sized>(&self, window: &W) -> bool {
        let Some(hwnd) = win32_hwnd(window) else {
            return false;
        };
        let mut mouse = windows_sys::Win32::Foundation::POINT::default();
        if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut mouse) } == 0 {
            return false;
        }
        let next = live_frame_after_delta(
            [self.origin_x, self.origin_y, self.width, self.height],
            f64::from(mouse.x) - self.mouse_x,
            f64::from(mouse.y) - self.mouse_y,
            self.edge,
            self.min,
            true,
        );
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                next[0] as i32,
                next[1] as i32,
                next[2] as i32,
                next[3] as i32,
                windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            );
        }
        true
    }

    pub fn end<W: HasWindowHandle + ?Sized>(self, _window: &W) {
        unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
        }
    }
}

/// Captures a window origin so later pointer moves translate the frame
/// without a nested OS move loop.
///
/// This is [`LiveFrameResize`]'s counterpart for position, and the reason it
/// exists beside [`drag_custom_title_bar`]: that one hands the gesture to the
/// platform, which only understands a held primary button. AppKit ignores a
/// window drag whose current event is not a left press or drag, and Win32
/// enters the caption move loop, which keeps following the cursor until a
/// primary release arrives — a gesture held with any other button would
/// either do nothing or never let go. Nothing here reads the platform's
/// current event or fakes a caption press; it samples the cursor, so any
/// button drives it and the host decides when it ends.
///
/// Mouse capture is the caller's: winit takes it on every button press and
/// drops it on the matching release, so a gesture that leaves the window
/// keeps reporting moves without this type touching capture at all.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug, Clone, Copy)]
pub struct LiveFrameMove {
    origin_x: f64,
    origin_y: f64,
    mouse_x: f64,
    mouse_y: f64,
}

#[cfg(target_os = "macos")]
impl LiveFrameMove {
    pub fn begin<W: HasWindowHandle + ?Sized>(window: &W) -> Option<Self> {
        let window = appkit_window(window)?;
        let origin = window.frame().origin;
        let mouse = objc2_app_kit::NSEvent::mouseLocation();
        Some(Self {
            origin_x: origin.x,
            origin_y: origin.y,
            mouse_x: mouse.x,
            mouse_y: mouse.y,
        })
    }

    pub fn update<W: HasWindowHandle + ?Sized>(&self, window: &W) -> bool {
        // Screen points and the frame origin share their axes on AppKit, so
        // the cursor delta is the origin delta.
        let mouse = objc2_app_kit::NSEvent::mouseLocation();
        self.set_origin(
            window,
            self.origin_x + (mouse.x - self.mouse_x),
            self.origin_y + (mouse.y - self.mouse_y),
        )
    }

    /// Puts the window back where the gesture started.
    pub fn cancel<W: HasWindowHandle + ?Sized>(&self, window: &W) -> bool {
        self.set_origin(window, self.origin_x, self.origin_y)
    }

    fn set_origin<W: HasWindowHandle + ?Sized>(&self, window: &W, x: f64, y: f64) -> bool {
        let Some(window) = appkit_window(window) else {
            return false;
        };
        window.setFrameOrigin(objc2_foundation::NSPoint::new(x, y));
        true
    }
}

#[cfg(target_os = "windows")]
impl LiveFrameMove {
    pub fn begin<W: HasWindowHandle + ?Sized>(window: &W) -> Option<Self> {
        let hwnd = win32_hwnd(window)?;
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        let mut mouse = windows_sys::Win32::Foundation::POINT::default();
        unsafe {
            if windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) == 0 {
                return None;
            }
            if windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut mouse) == 0 {
                return None;
            }
        }
        Some(Self {
            origin_x: f64::from(rect.left),
            origin_y: f64::from(rect.top),
            mouse_x: f64::from(mouse.x),
            mouse_y: f64::from(mouse.y),
        })
    }

    pub fn update<W: HasWindowHandle + ?Sized>(&self, window: &W) -> bool {
        // Cursor and window rect are both screen pixels, so the cursor delta
        // is the origin delta.
        let mut mouse = windows_sys::Win32::Foundation::POINT::default();
        if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut mouse) } == 0 {
            return false;
        }
        self.set_origin(
            window,
            self.origin_x + (f64::from(mouse.x) - self.mouse_x),
            self.origin_y + (f64::from(mouse.y) - self.mouse_y),
        )
    }

    /// Puts the window back where the gesture started.
    pub fn cancel<W: HasWindowHandle + ?Sized>(&self, window: &W) -> bool {
        self.set_origin(window, self.origin_x, self.origin_y)
    }

    fn set_origin<W: HasWindowHandle + ?Sized>(&self, window: &W, x: f64, y: f64) -> bool {
        let Some(hwnd) = win32_hwnd(window) else {
            return false;
        };
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                x as i32,
                y as i32,
                0,
                0,
                windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            );
        }
        true
    }
}

/// Centers macOS traffic lights inside a custom titlebar `titlebar_height`
/// logical points tall: the system keeps them centered in the standard
/// titlebar strip, which reads high inside NanaUI's taller bar. The
/// horizontal position stays the system's until the title bar lays out a
/// placeholder ([`place_native_window_controls`]). Other platforms have no
/// native controls and always succeed.
fn center_traffic_lights<W: HasWindowHandle + ?Sized>(window: &W, titlebar_height: f64) -> bool {
    move_traffic_lights(window, None, titlebar_height / 2.0)
}

/// Centers the native window buttons (macOS traffic lights) on a rectangle
/// in window logical points from the top-left corner, normally the layout box
/// of the title bar's window-controls placeholder.
///
/// Any rectangle inside the window works — leading, inset, lower, or further
/// in — and the buttons keep their own size and spacing; only the center of
/// the cluster follows. A rectangle that reaches outside the window moves
/// them out of the frame that draws and hits them, so callers keep it within
/// the window. Windows that keep the system caption own their own buttons
/// and must not call this.
///
/// The move shifts the whole titlebar view the buttons live in. A frame set
/// on a button itself is dropped, because AppKit lays the buttons out under
/// constraints, and shifting only their group leaves it outside its parent's
/// bounds, where the buttons still draw but no longer take clicks. AppKit
/// puts the titlebar back whenever it lays it out again — when the buttons
/// are shown, hidden, faded, or the window changes style — so hosts call
/// this every frame and right after those events. Other platforms have no
/// native controls and always succeed.
pub fn place_native_window_controls<W: HasWindowHandle + ?Sized>(
    window: &W,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> bool {
    move_traffic_lights(window, Some(x + width / 2.0), y + height / 2.0)
}

/// Moves the traffic-light cluster so its center lands on `center_x` (kept
/// when `None`) and `center_y`, measured from the window's top-left corner.
#[cfg(target_os = "macos")]
fn move_traffic_lights<W: HasWindowHandle + ?Sized>(
    window: &W,
    center_x: Option<f64>,
    center_y: f64,
) -> bool {
    use objc2_app_kit::NSWindowButton;
    use objc2_foundation::NSPoint;

    let Some(window) = appkit_window(window) else {
        return false;
    };
    // All three buttons must exist and share one container view, so the
    // buttons keep their spacing while the titlebar around them moves.
    let (Some(close), Some(miniaturize), Some(zoom)) = (
        window.standardWindowButton(NSWindowButton::CloseButton),
        window.standardWindowButton(NSWindowButton::MiniaturizeButton),
        window.standardWindowButton(NSWindowButton::ZoomButton),
    ) else {
        return false;
    };
    // SAFETY: plain property reads on live views owned by this window.
    let container = unsafe { close.superview() };
    // The titlebar view is what moves, not the button group inside it: a
    // group shifted past its parent's bounds keeps drawing but stops being
    // hit, and the buttons would no longer answer clicks.
    // SAFETY: plain property reads on live views owned by this window.
    let titlebar = container
        .as_ref()
        .and_then(|view| unsafe { view.superview() });
    // SAFETY: plain property reads on live views owned by this window.
    let titlebar_parent = titlebar
        .as_ref()
        .and_then(|view| unsafe { view.superview() });
    let (Some(container), Some(titlebar), Some(titlebar_parent)) =
        (container, titlebar, titlebar_parent)
    else {
        return false;
    };
    // SAFETY: plain property reads on live views owned by this window.
    if unsafe { miniaturize.superview() }.as_ref() != Some(&container)
        || unsafe { zoom.superview() }.as_ref() != Some(&container)
    {
        return false;
    }

    // Buttons that are fading or hidden are mid-flight: AppKit is moving
    // them itself, and a position read now sends the cluster somewhere else.
    // Leaving them be is safe — a reveal restores them at full alpha, and
    // the window places them again then.
    if [&close, &miniaturize, &zoom]
        .into_iter()
        .any(|button| button.isHidden() || button.alphaValue() < 1.0)
    {
        return true;
    }

    // Current cluster center in window space. Window base coordinates grow
    // upward from the bottom-left corner.
    let close_in_window = close.convertRect_toView(close.bounds(), None);
    let zoom_in_window = zoom.convertRect_toView(zoom.bounds(), None);
    let width = zoom_in_window.origin.x + zoom_in_window.size.width - close_in_window.origin.x;
    if width <= 0.0 || close_in_window.size.height <= 0.0 {
        return false;
    }
    let current_x = close_in_window.origin.x + width / 2.0;
    let current_from_top =
        window.frame().size.height - (close_in_window.origin.y + close_in_window.size.height / 2.0);
    let delta_x = center_x.map_or(0.0, |x| x - current_x);
    let delta_y = center_y - current_from_top;
    if delta_x.abs() < 0.5 && delta_y.abs() < 0.5 {
        return true;
    }

    // Learn how the titlebar's parent maps its axes onto window space
    // (titlebar views are flipped, the window is not) instead of assuming.
    let origin = titlebar_parent.convertPoint_toView(NSPoint::new(0.0, 0.0), None);
    let unit_x = titlebar_parent.convertPoint_toView(NSPoint::new(1.0, 0.0), None);
    let unit_y = titlebar_parent.convertPoint_toView(NSPoint::new(0.0, 1.0), None);
    let x_per_unit = unit_x.x - origin.x;
    let y_up_per_unit = unit_y.y - origin.y;
    if x_per_unit == 0.0 || y_up_per_unit == 0.0 {
        return false;
    }
    let mut frame = titlebar.frame();
    frame.origin.x += delta_x / x_per_unit;
    frame.origin.y -= delta_y / y_up_per_unit;
    titlebar.setFrameOrigin(frame.origin);
    true
}

#[cfg(not(target_os = "macos"))]
fn move_traffic_lights<W: HasWindowHandle + ?Sized>(
    _window: &W,
    _center_x: Option<f64>,
    _center_y: f64,
) -> bool {
    true
}

/// Fades the native window buttons (macOS traffic lights) over `duration`.
/// Hidden buttons leave paint, hover and clicks once the fade ends; a reveal
/// that arrives mid-fade takes over from the presented alpha. Platforms whose
/// window buttons are `AppTitleBar` controls have no native buttons and
/// always succeed.
pub fn set_native_window_controls_visible<W: HasWindowHandle + ?Sized>(
    window: &W,
    visible: bool,
    duration: std::time::Duration,
) -> bool {
    #[cfg(target_os = "macos")]
    {
        fade_traffic_lights(window, visible, duration)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, visible, duration);
        true
    }
}

#[cfg(target_os = "macos")]
fn fade_traffic_lights<W: HasWindowHandle + ?Sized>(
    window: &W,
    visible: bool,
    duration: std::time::Duration,
) -> bool {
    use block2::RcBlock;
    use objc2_app_kit::{NSAnimatablePropertyContainer, NSAnimationContext, NSWindowButton};

    let Some(window) = appkit_window(window) else {
        return false;
    };
    let buttons: Vec<_> = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ]
    .into_iter()
    .filter_map(|kind| window.standardWindowButton(kind))
    .collect();
    if buttons.is_empty() {
        return false;
    }
    let alpha = if visible { 1.0 } else { 0.0 };
    if duration.is_zero() {
        for button in &buttons {
            button.setAlphaValue(alpha);
            button.setHidden(!visible);
        }
        return true;
    }
    if visible {
        for button in &buttons {
            button.setHidden(false);
        }
    }
    let seconds = duration.as_secs_f64();
    let changes = RcBlock::new(|context: std::ptr::NonNull<NSAnimationContext>| {
        // SAFETY: AppKit passes the live context of this animation group.
        unsafe { context.as_ref() }.setDuration(seconds);
        for button in &buttons {
            button.animator().setAlphaValue(alpha);
        }
    });
    // The model alpha is the latest target, so a reveal issued during this
    // fade keeps the buttons.
    let concealed = buttons.clone();
    let completion = RcBlock::new(move || {
        for button in &concealed {
            if button.alphaValue() <= 0.0 {
                button.setHidden(true);
            }
        }
    });
    NSAnimationContext::runAnimationGroup_completionHandler(
        &changes,
        (!visible).then_some(&*completion),
    );
    true
}

#[cfg(target_os = "macos")]
fn set_drag_enabled<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> bool {
    let Some(window) = appkit_window(window) else {
        return false;
    };
    window.setMovable(enabled);
    true
}

#[cfg(target_os = "macos")]
fn drag<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEventType};

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = appkit_window(window) else {
        return false;
    };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
        return false;
    };
    // AppKit silently ignores a drag started from any other event, e.g. when
    // the press was queued and handled outside its own mouseDown dispatch.
    let kind = event.r#type();
    if kind != NSEventType::LeftMouseDown && kind != NSEventType::LeftMouseDragged {
        return false;
    }

    window.setMovable(true);
    window.performWindowDragWithEvent(&event);
    window.setMovable(false);
    true
}

#[cfg(target_os = "macos")]
fn appkit_window<W: HasWindowHandle + ?Sized>(
    window: &W,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2_app_kit::NSView;
    use raw_window_handle::RawWindowHandle;

    let Ok(handle) = window.window_handle() else {
        return None;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

// Used by the macOS and Windows live-resize paths; on other targets only the
// tests reach it. Same idiom as the two helpers at the top of this file.
#[cfg_attr(
    not(any(test, target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
fn live_frame_after_delta(
    start: [f64; 4],
    dx: f64,
    dy: f64,
    edge: FrameResizeEdge,
    min: (f64, f64),
    y_down: bool,
) -> [f64; 4] {
    let [mut x, mut y, mut width, mut height] = start;
    let west = matches!(
        edge,
        FrameResizeEdge::West | FrameResizeEdge::NorthWest | FrameResizeEdge::SouthWest
    );
    let east = matches!(
        edge,
        FrameResizeEdge::East | FrameResizeEdge::NorthEast | FrameResizeEdge::SouthEast
    );
    let south = matches!(
        edge,
        FrameResizeEdge::South | FrameResizeEdge::SouthEast | FrameResizeEdge::SouthWest
    );
    let north = matches!(
        edge,
        FrameResizeEdge::North | FrameResizeEdge::NorthEast | FrameResizeEdge::NorthWest
    );
    if east {
        width += dx;
    } else if west {
        x += dx;
        width -= dx;
    }
    if y_down {
        if south {
            height += dy;
        } else if north {
            y += dy;
            height -= dy;
        }
    } else if north {
        height += dy;
    } else if south {
        y += dy;
        height -= dy;
    }
    let right = x + width;
    let bottom = y + height;
    width = width.max(min.0);
    height = height.max(min.1);
    if west {
        x = right - width;
    }
    if (y_down && north) || (!y_down && south) {
        y = bottom - height;
    }
    [x, y, width, height]
}

#[cfg(not(target_os = "macos"))]
fn set_drag_enabled<W: HasWindowHandle + ?Sized>(_window: &W, _enabled: bool) -> bool {
    true
}

#[cfg(target_os = "windows")]
fn win32_hwnd<W: HasWindowHandle + ?Sized>(
    window: &W,
) -> Option<windows_sys::Win32::Foundation::HWND> {
    use raw_window_handle::RawWindowHandle;

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get() as windows_sys::Win32::Foundation::HWND)
}

#[cfg(target_os = "windows")]
fn win32_min_track_size(hwnd: windows_sys::Win32::Foundation::HWND) -> (f64, f64) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MINMAXINFO, SendMessageW, WM_GETMINMAXINFO};

    let mut info = MINMAXINFO::default();
    unsafe {
        SendMessageW(
            hwnd,
            WM_GETMINMAXINFO,
            0,
            std::ptr::from_mut(&mut info) as isize,
        );
    }
    // The pinned winit proc fills only `ptMinTrackSize` (from the window's
    // min size) and never calls `DefWindowProc`, so `ptMaxTrackSize` stays at
    // its default of zero. Only the min is meaningful here.
    (
        f64::from(info.ptMinTrackSize.x),
        f64::from(info.ptMinTrackSize.y),
    )
}

#[cfg(target_os = "windows")]
fn drag<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::HTCAPTION;
    send_nc_lbutton_down(window, HTCAPTION as usize)
}

#[cfg(target_os = "windows")]
fn resize_windows<W: HasWindowHandle + ?Sized>(window: &W, edge: FrameResizeEdge) -> bool {
    send_nc_lbutton_down(window, hit_test_for_edge(edge))
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn hit_test_for_edge(edge: FrameResizeEdge) -> usize {
    // Win32 HT* values: left=10, right=11, top=12, top-left=13, top-right=14,
    // bottom=15, bottom-left=16, bottom-right=17.
    match edge {
        FrameResizeEdge::West => 10,
        FrameResizeEdge::East => 11,
        FrameResizeEdge::North => 12,
        FrameResizeEdge::NorthWest => 13,
        FrameResizeEdge::NorthEast => 14,
        FrameResizeEdge::South => 15,
        FrameResizeEdge::SouthWest => 16,
        FrameResizeEdge::SouthEast => 17,
    }
}

#[cfg(target_os = "windows")]
fn send_nc_lbutton_down<W: HasWindowHandle + ?Sized>(window: &W, hit: usize) -> bool {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, SendMessageW, WM_NCLBUTTONDOWN,
    };

    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get() as HWND;
    unsafe {
        ReleaseCapture();
        let mut point = POINT { x: 0, y: 0 };
        let lparam = if GetCursorPos(&mut point) != 0 {
            ((point.y as u32) << 16 | (point.x as u32 & 0xFFFF)) as isize
        } else {
            0
        };
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, hit, lparam);
    }
    true
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn drag<W: HasWindowHandle + ?Sized>(_window: &W) -> bool {
    false
}

#[cfg(target_os = "windows")]
const DWMWA_BORDER_COLOR: u32 = 34;
#[cfg(target_os = "windows")]
const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
#[cfg(target_os = "windows")]
const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const fn dwm_corner_preference(rounded_corners: bool) -> i32 {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Graphics::Dwm::{DWMWCP_DONOTROUND, DWMWCP_ROUND};
        if rounded_corners {
            DWMWCP_ROUND
        } else {
            DWMWCP_DONOTROUND
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if rounded_corners { 1 } else { 0 }
    }
}

/// Rounded windows take the system stroke Windows 11 pairs with the round
/// clip; square ones take none, so DWM leaves the HWND rectangle unpainted.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const fn dwm_border_color(rounded_corners: bool) -> u32 {
    #[cfg(target_os = "windows")]
    {
        if rounded_corners {
            DWMWA_COLOR_DEFAULT
        } else {
            DWMWA_COLOR_NONE
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if rounded_corners {
            0xFFFF_FFFF
        } else {
            0xFFFF_FFFE
        }
    }
}

#[cfg(target_os = "windows")]
fn apply_window_shape<W: HasWindowHandle + ?Sized>(window: &W, rounded_corners: bool) -> bool {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute,
    };

    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get() as HWND;
    let preference: DWM_WINDOW_CORNER_PREFERENCE = dwm_corner_preference(rounded_corners);
    let corner_ok = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            std::ptr::from_ref(&preference).cast(),
            std::mem::size_of_val(&preference) as u32,
        )
    } >= 0;
    // Both branches write the attribute: a window that flips material at
    // runtime would otherwise keep whichever stroke it was last given.
    let color = dwm_border_color(rounded_corners);
    let border_ok = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            std::ptr::from_ref(&color).cast(),
            std::mem::size_of_val(&color) as u32,
        )
    } >= 0;
    corner_ok && border_ok
}

#[cfg(target_os = "windows")]
const DWMWA_NCRENDERING_POLICY: u32 = 2;
/// `DWMNCRP_ENABLED`.
#[cfg(target_os = "windows")]
const DWMNCRP_ENABLED: u32 = 2;
/// `DWMNCRP_DISABLED`.
#[cfg(target_os = "windows")]
const DWMNCRP_DISABLED: u32 = 1;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const fn nc_rendering_policy(enabled: bool) -> u32 {
    #[cfg(target_os = "windows")]
    {
        if enabled {
            DWMNCRP_ENABLED
        } else {
            DWMNCRP_DISABLED
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if enabled { 2 } else { 1 }
    }
}

#[cfg(target_os = "windows")]
fn set_nc_rendering_policy<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> bool {
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;

    let Some(hwnd) = chrome_hwnd(window) else {
        return false;
    };
    let policy = nc_rendering_policy(enabled);
    // SAFETY: `hwnd` belongs to the live window this call was handed, and the
    // attribute is read from a `u32` this frame owns.
    let applied = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            std::ptr::from_ref(&policy).cast(),
            std::mem::size_of_val(&policy) as u32,
        )
    };
    applied >= 0
}

/// Subclass identity for the `WM_STYLECHANGING` guard.
#[cfg(target_os = "windows")]
const STYLE_GUARD_SUBCLASS_ID: usize = 0x4E_41_53_47;

/// The bits the guard takes out of every style written to this window.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
const fn frameless_mask(frameless: bool) -> isize {
    if frameless { FRAMELESS_STYLES } else { 0 }
}

#[cfg(target_os = "windows")]
fn chrome_hwnd<W: HasWindowHandle + ?Sized>(
    window: &W,
) -> Option<windows_sys::Win32::Foundation::HWND> {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::HWND;

    let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get() as HWND)
}

#[cfg(target_os = "windows")]
fn set_frameless_style<W: HasWindowHandle + ?Sized>(
    window: &W,
    frameless: bool,
    resizable: bool,
) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
    };

    let Some(hwnd) = chrome_hwnd(window) else {
        return false;
    };
    // Arm the guard before writing the style, so the bits cannot return in the
    // gap: winit rewrites the whole style from its own flags, and does it from
    // inside its WndProc too, where no host call wraps it.
    let guarded = install_style_guard(hwnd, frameless_mask(frameless));
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let next = client_chrome_style(current, frameless, resizable);
        if next != current {
            SetWindowLongPtrW(hwnd, GWL_STYLE, next);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
    guarded
}

/// Masks `mask` out of every `GWL_STYLE` write this window receives.
///
/// winit derives the whole style from its own `WindowFlags` and writes it back
/// on every change — including from its WndProc on `WM_DPICHANGED`, a path the
/// host never sees. Re-applying the strip after each host call cannot cover
/// that one, so the strip belongs on the message instead.
#[cfg(target_os = "windows")]
fn install_style_guard(hwnd: windows_sys::Win32::Foundation::HWND, mask: isize) -> bool {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;

    // `SetWindowSubclass` is idempotent for a given (window, proc, id) triple
    // and replaces the reference data in place, so re-arming with a new mask
    // swaps the mask rather than stacking a second subclass — the host arms
    // this on every chrome reconciliation and must not grow a chain of them.
    // `mask == 0` makes the callback a pass-through, which is what an opaque
    // window wants. The subclass removes itself on `WM_NCDESTROY`.
    // SAFETY: `hwnd` belongs to the live window this call was handed.
    unsafe {
        SetWindowSubclass(
            hwnd,
            Some(style_guard_proc),
            STYLE_GUARD_SUBCLASS_ID,
            mask as usize,
        ) != 0
    }
}

/// Keeps NanaUI's frame invariant on `WM_STYLECHANGING` without owning the
/// message.
///
/// A framework must not take `WM_STYLECHANGING` away from the rest of the
/// window: a native extension, an accessibility shim or an embedding host may
/// have its own subclass on the same HWND, and returning early would cut every
/// one of them — and the original WindowProc — out of a message they are
/// entitled to see. So the message is always forwarded down the chain first,
/// and the mask is applied to whatever the chain left in `styleNew`.
///
/// Applying it *after* `DefSubclassProc` is what makes it the final word over
/// everything installed below this guard, which is where winit's own proc
/// sits. A subclass installed after this one runs before it and gets control
/// back afterwards, so it can still write the bits back; that is inherent to
/// Win32 subclass ordering and is the price of not owning the message. Arming
/// the guard again re-asserts it on the next style write.
#[cfg(target_os = "windows")]
unsafe extern "system" fn style_guard_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    id: usize,
    mask: usize,
) -> isize {
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, STYLESTRUCT, WM_NCDESTROY, WM_STYLECHANGING,
    };

    if message == WM_NCDESTROY {
        // Detach before forwarding, which is the documented order: the chain
        // stays usable for the rest of this call, and the window does not
        // finish being destroyed with a subclass still pointing here.
        // SAFETY: removing the subclass this callback was invoked as.
        unsafe {
            RemoveWindowSubclass(hwnd, Some(style_guard_proc), id);
        }
    }
    // SAFETY: forwarding the message this callback was given, unchanged.
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if message == WM_STYLECHANGING && wparam == GWL_STYLE as usize && mask != 0 && lparam != 0 {
        // SAFETY: `WM_STYLECHANGING` passes a `STYLESTRUCT` the handler is
        // expected to edit in place, and the sender owns it for the call.
        let style = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
        style.styleNew = guarded_style(style.styleNew as isize, mask as isize) as u32;
    }
    result
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "windows")]
    use super::install_style_guard;
    use super::{
        FRAMELESS_STYLES, FrameResizeEdge, WS_CAPTION, WS_SYSMENU, WS_THICKFRAME,
        client_chrome_style, dwm_border_color, dwm_corner_preference, frameless_mask,
        guarded_style, hit_test_for_edge, live_frame_after_delta,
    };

    const WS_BORDER: isize = 0x0080_0000;
    const WS_CLIPSIBLINGS: isize = 0x0400_0000;
    const WS_MINIMIZEBOX: isize = 0x0002_0000;
    const WS_VISIBLE: isize = 0x1000_0000;

    #[test]
    fn overlay_shape_requests_square_corners() {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::Graphics::Dwm::{DWMWCP_DONOTROUND, DWMWCP_ROUND};
            assert_eq!(dwm_corner_preference(true), DWMWCP_ROUND);
            assert_eq!(dwm_corner_preference(false), DWMWCP_DONOTROUND);
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(dwm_corner_preference(true), 1);
            assert_eq!(dwm_corner_preference(false), 0);
        }
    }

    /// A window whose material flips at runtime asks for both, so neither
    /// branch may leave the previous stroke in place.
    #[test]
    fn overlay_shape_drops_the_stroke_a_rounded_window_restores() {
        assert_eq!(dwm_border_color(false), 0xFFFF_FFFE);
        assert_eq!(dwm_border_color(true), 0xFFFF_FFFF);
    }

    /// DWM renders the caption buttons and the drop shadow for any HWND that
    /// still looks framed, so a transparent client has to lose every frame bit
    /// rather than just the caption.
    #[test]
    fn client_chrome_style_strips_every_frame_bit_and_keeps_the_rest() {
        let current =
            WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPSIBLINGS | WS_VISIBLE;
        let frameless = client_chrome_style(current, true, true);
        assert_eq!(frameless & FRAMELESS_STYLES, 0);
        assert_eq!(frameless & WS_BORDER, 0);
        assert_eq!(frameless & WS_MINIMIZEBOX, WS_MINIMIZEBOX);
        assert_eq!(frameless & WS_CLIPSIBLINGS, WS_CLIPSIBLINGS);
        assert_eq!(frameless & WS_VISIBLE, WS_VISIBLE);
        assert_eq!(client_chrome_style(frameless, false, true), current);
    }

    /// winit only sets `WS_THICKFRAME` for a resizable window, so restoring it
    /// unconditionally would grow a system resize border the window never had.
    #[test]
    fn restoring_a_fixed_size_window_leaves_the_resize_border_off() {
        let frameless = WS_MINIMIZEBOX | WS_CLIPSIBLINGS | WS_VISIBLE;
        let restored = client_chrome_style(frameless, false, false);
        assert_eq!(restored & WS_CAPTION, WS_CAPTION);
        assert_eq!(restored & WS_SYSMENU, WS_SYSMENU);
        assert_eq!(restored & WS_THICKFRAME, 0);
    }

    /// An opaque window arms the guard with an empty mask rather than removing
    /// the subclass, so the mask has to be able to say "take nothing".
    #[test]
    fn the_mask_is_empty_for_a_window_that_keeps_its_frame() {
        assert_eq!(frameless_mask(true), FRAMELESS_STYLES);
        assert_eq!(frameless_mask(false), 0);
    }

    /// The guard runs on every `GWL_STYLE` write the window receives, winit's
    /// included, so it must take out the frame bits and nothing else.
    #[test]
    fn style_guard_masks_the_frame_bits_and_passes_the_rest_through() {
        let winit_style =
            WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPSIBLINGS | WS_VISIBLE;
        let guarded = guarded_style(winit_style, FRAMELESS_STYLES);
        assert_eq!(guarded, WS_MINIMIZEBOX | WS_CLIPSIBLINGS | WS_VISIBLE);
        // An opaque window arms the guard with an empty mask instead of
        // removing the subclass, so that case has to stay a pass-through.
        assert_eq!(guarded_style(winit_style, 0), winit_style);
    }

    /// NanaUI is a framework on somebody else's window: a native extension,
    /// an accessibility shim or an embedding host may hold its own
    /// `WM_STYLECHANGING` subclass on the same HWND. The guard must not take
    /// the message away from them, and its invariant must still be the one
    /// that survives.
    ///
    /// Runs on a real HWND because the thing under test is Win32 subclass
    /// chaining, which has no model worth asserting against.
    #[cfg(target_os = "windows")]
    #[test]
    fn the_style_guard_shares_wm_stylechanging_with_another_subclass() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_STYLE, RegisterClassW, STYLESTRUCT,
            SendMessageW, WM_STYLECHANGING, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        };

        /// How many `WM_STYLECHANGING` messages the other subclass saw, and
        /// the style it observed. A test-local static is enough: the fixture
        /// owns the only window that carries this subclass.
        static OBSERVED: AtomicUsize = AtomicUsize::new(0);
        static OBSERVED_STYLE: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "system" fn other_subclass(
            hwnd: HWND,
            message: u32,
            wparam: usize,
            lparam: isize,
            _id: usize,
            _data: usize,
        ) -> isize {
            if message == WM_STYLECHANGING && lparam != 0 {
                OBSERVED.fetch_add(1, Ordering::Relaxed);
                // SAFETY: the sender owns this STYLESTRUCT for the call.
                let style = unsafe { &*(lparam as *const STYLESTRUCT) };
                OBSERVED_STYLE.store(style.styleNew as usize, Ordering::Relaxed);
            }
            // SAFETY: forwarding the message unchanged, as a subclass must.
            unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }

        fn wide(text: &str) -> Vec<u16> {
            text.encode_utf16().chain(std::iter::once(0)).collect()
        }

        let class_name = wide("NanaStyleGuardFixture");
        let class = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            lpszClassName: class_name.as_ptr(),
            // SAFETY: `WNDCLASSW` is a plain C struct of integers and pointers
            // whose all-zero state is the documented "unset" one; the fields
            // this fixture cares about are set above.
            ..unsafe { std::mem::zeroed() }
        };
        // SAFETY: a plain class registration with a static name and DefWindowProcW.
        unsafe {
            RegisterClassW(&class);
        }
        // SAFETY: creating a hidden top-level window of the class just registered.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                wide("fixture").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                64,
                64,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert!(!hwnd.is_null(), "fixture window");

        // The other subclass is installed first, so the guard sits above it
        // and its `DefSubclassProc` is what has to reach it.
        // SAFETY: `hwnd` is the live fixture window.
        assert!(unsafe { SetWindowSubclass(hwnd, Some(other_subclass), 1, 0) } != 0);
        assert!(install_style_guard(hwnd, FRAMELESS_STYLES));

        let mut style = STYLESTRUCT {
            styleOld: 0,
            styleNew: (WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_CLIPSIBLINGS) as u32,
        };
        // SAFETY: sending a style change with a STYLESTRUCT this frame owns.
        unsafe {
            SendMessageW(
                hwnd,
                WM_STYLECHANGING,
                GWL_STYLE as usize,
                std::ptr::from_mut(&mut style) as isize,
            );
        }

        assert_eq!(
            OBSERVED.load(Ordering::Relaxed),
            1,
            "the other subclass must still receive WM_STYLECHANGING"
        );
        assert_eq!(
            OBSERVED_STYLE.load(Ordering::Relaxed) as isize & FRAMELESS_STYLES,
            FRAMELESS_STYLES,
            "it must see the style as written, not one the guard already edited"
        );
        assert_eq!(
            style.styleNew as isize & FRAMELESS_STYLES,
            0,
            "and NanaUI's invariant is still the final word"
        );
        assert_eq!(
            style.styleNew as isize & WS_CLIPSIBLINGS,
            WS_CLIPSIBLINGS,
            "bits outside the mask pass through untouched"
        );

        // Re-arming swaps the mask in place instead of stacking a second
        // subclass, so the chain the host reconciles every frame stays one
        // link long.
        assert!(install_style_guard(hwnd, 0));
        OBSERVED.store(0, Ordering::Relaxed);
        let mut opaque = STYLESTRUCT {
            styleOld: 0,
            styleNew: (WS_CAPTION | WS_THICKFRAME | WS_SYSMENU) as u32,
        };
        // SAFETY: as above.
        unsafe {
            SendMessageW(
                hwnd,
                WM_STYLECHANGING,
                GWL_STYLE as usize,
                std::ptr::from_mut(&mut opaque) as isize,
            );
        }
        assert_eq!(OBSERVED.load(Ordering::Relaxed), 1);
        assert_eq!(
            opaque.styleNew as isize & FRAMELESS_STYLES,
            FRAMELESS_STYLES,
            "an empty mask is a pass-through, not a second strip"
        );

        // SAFETY: destroying the window this test created; the guard detaches
        // itself on WM_NCDESTROY.
        unsafe {
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn live_frame_grows_and_clamps_to_min_from_each_edge() {
        let start = [100.0, 200.0, 400.0, 300.0];
        let min = (120.0, 80.0);
        let cases = [
            (
                40.0,
                0.0,
                FrameResizeEdge::East,
                [100.0, 200.0, 440.0, 300.0],
            ),
            (
                40.0,
                0.0,
                FrameResizeEdge::West,
                [140.0, 200.0, 360.0, 300.0],
            ),
            (
                0.0,
                30.0,
                FrameResizeEdge::North,
                [100.0, 200.0, 400.0, 330.0],
            ),
            (
                0.0,
                30.0,
                FrameResizeEdge::South,
                [100.0, 230.0, 400.0, 270.0],
            ),
            (
                350.0,
                0.0,
                FrameResizeEdge::West,
                [380.0, 200.0, 120.0, 300.0],
            ),
            (
                500.0,
                400.0,
                FrameResizeEdge::NorthEast,
                [100.0, 200.0, 900.0, 700.0],
            ),
        ];
        for (dx, dy, edge, expected) in cases {
            assert_eq!(
                live_frame_after_delta(start, dx, dy, edge, min, false),
                expected
            );
        }
        assert_eq!(
            live_frame_after_delta(start, 0.0, 30.0, FrameResizeEdge::South, min, true),
            [100.0, 200.0, 400.0, 330.0]
        );
        assert_eq!(
            live_frame_after_delta(start, 0.0, 30.0, FrameResizeEdge::North, min, true),
            [100.0, 230.0, 400.0, 270.0]
        );
        // Growth is unbounded: a live drag must never clamp to a stale max.
        assert_eq!(
            live_frame_after_delta(start, 2_000.0, 0.0, FrameResizeEdge::East, min, true),
            [100.0, 200.0, 2_400.0, 300.0]
        );
        assert_eq!(hit_test_for_edge(FrameResizeEdge::West), 10);
        assert_eq!(hit_test_for_edge(FrameResizeEdge::SouthEast), 17);
    }
}
