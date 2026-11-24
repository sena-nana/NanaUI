#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod win32;

use raw_window_handle::HasWindowHandle;

#[cfg(target_os = "macos")]
pub(crate) use macos::appkit_movable_window;
#[cfg(target_os = "macos")]
pub use macos::native_window_maximized;
#[cfg(target_os = "windows")]
pub(crate) use win32::{
    guarded_style, reapply_window_shape, win32_min_track_size, win32_movable_hwnd,
};

/// Win32 `WS_CAPTION` (`WS_BORDER | WS_DLGFRAME`).
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(super) const WS_CAPTION: isize = 0x00C0_0000;
/// Win32 `WS_THICKFRAME`, spelled `WS_SIZEBOX` by winit.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(super) const WS_THICKFRAME: isize = 0x0004_0000;
/// Win32 `WS_SYSMENU`.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(super) const WS_SYSMENU: isize = 0x0008_0000;

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
pub(super) const FRAMELESS_STYLES: isize = WS_CAPTION | WS_THICKFRAME | WS_SYSMENU;

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
    let prepared = win32::apply_window_shape(window, rounded_corners) && prepared;
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
/// `docs/reference/window.md` for the comparison the default comes from.
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
        win32::set_nc_rendering_policy(window, enabled)
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
        win32::set_frameless_style(window, frameless, resizable)
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
        win32::arm_frameless_guard(window, frameless)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, frameless);
        true
    }
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
        win32::resize_windows(window, edge)
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
        macos::native_live_resize_active(window)
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
        macos::set_metal_present_transaction(window, enabled)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, enabled);
        false
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

fn move_traffic_lights<W: HasWindowHandle + ?Sized>(
    window: &W,
    center_x: Option<f64>,
    center_y: f64,
) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::move_traffic_lights(window, center_x, center_y)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, center_x, center_y);
        true
    }
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
        macos::fade_traffic_lights(window, visible, duration)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, visible, duration);
        true
    }
}

fn set_drag_enabled<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::set_drag_enabled(window, enabled)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, enabled);
        true
    }
}

fn drag<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::drag(window)
    }
    #[cfg(target_os = "windows")]
    {
        win32::drag(window)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = window;
        false
    }
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(super) const fn client_chrome_style(current: isize, frameless: bool, resizable: bool) -> isize {
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
pub(super) const fn guarded_style_bits(style_new: isize, mask: isize) -> isize {
    style_new & !mask
}

/// The bits the guard takes out of every style written to this window.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(super) const fn frameless_mask(frameless: bool) -> isize {
    if frameless { FRAMELESS_STYLES } else { 0 }
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(super) fn hit_test_for_edge(edge: FrameResizeEdge) -> usize {
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
            win32::DWMWA_COLOR_DEFAULT
        } else {
            win32::DWMWA_COLOR_NONE
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

#[cfg(test)]
mod tests {
    use super::{
        FRAMELESS_STYLES, FrameResizeEdge, WS_CAPTION, WS_SYSMENU, WS_THICKFRAME,
        client_chrome_style, dwm_border_color, dwm_corner_preference, frameless_mask,
        guarded_style_bits, hit_test_for_edge,
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
        let guarded = guarded_style_bits(winit_style, FRAMELESS_STYLES);
        assert_eq!(guarded, WS_MINIMIZEBOX | WS_CLIPSIBLINGS | WS_VISIBLE);
        // An opaque window arms the guard with an empty mask instead of
        // removing the hook, so that case has to stay a pass-through.
        assert_eq!(guarded_style_bits(winit_style, 0), winit_style);
    }

    #[test]
    fn hit_test_maps_frame_edges() {
        assert_eq!(hit_test_for_edge(FrameResizeEdge::West), 10);
        assert_eq!(hit_test_for_edge(FrameResizeEdge::SouthEast), 17);
    }
}
