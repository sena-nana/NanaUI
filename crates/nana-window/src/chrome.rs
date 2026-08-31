use raw_window_handle::HasWindowHandle;

/// Win32 `WS_CAPTION` (`WS_BORDER | WS_DLGFRAME`).
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
const WS_CAPTION: isize = 0x00C0_0000;

/// Prepares native titlebar dragging and client-chrome window shape.
pub fn prepare_client_chrome<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    let prepared = prepare_custom_title_bar(window);
    #[cfg(target_os = "windows")]
    let prepared = apply_rounded_corners(window) && prepared;
    prepared
}

/// Clears the Win32 caption frame so a transparent client-chrome window is not
/// left with `WS_CAPTION`. Other platforms no-op.
pub fn suppress_system_caption<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    #[cfg(target_os = "windows")]
    {
        clear_caption_style(window)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        true
    }
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn client_chrome_style_without_caption(current: isize) -> isize {
    current & !WS_CAPTION
}

/// Prepares native titlebar dragging for NanaUI's custom titlebar regions.
pub fn prepare_custom_title_bar<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    set_drag_enabled(window, false)
}

/// Performs one explicit NanaUI titlebar drag.
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
    use objc2_app_kit::NSApplication;

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = appkit_window(window) else {
        return false;
    };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
        return false;
    };

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
fn apply_rounded_corners<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
        DwmSetWindowAttribute,
    };

    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get() as HWND;
    let preference: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND;
    let status = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            std::ptr::from_ref(&preference).cast(),
            std::mem::size_of_val(&preference) as u32,
        )
    };
    status >= 0
}

#[cfg(target_os = "windows")]
fn clear_caption_style<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
    };

    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = handle.hwnd.get() as HWND;
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let next = client_chrome_style_without_caption(current);
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
    true
}

#[cfg(test)]
mod tests {
    use super::{
        FrameResizeEdge, WS_CAPTION, client_chrome_style_without_caption, hit_test_for_edge,
        live_frame_after_delta,
    };

    const WS_BORDER: isize = 0x0080_0000;
    const WS_CLIPSIBLINGS: isize = 0x0400_0000;
    const WS_SYSMENU: isize = 0x0008_0000;
    const WS_THICKFRAME: isize = 0x0004_0000;
    const WS_VISIBLE: isize = 0x1000_0000;

    #[test]
    fn client_chrome_style_clears_caption_and_keeps_frame_bits() {
        let current = WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_CLIPSIBLINGS | WS_VISIBLE;
        let next = client_chrome_style_without_caption(current);
        assert_eq!(next & WS_CAPTION, 0);
        assert_eq!(next & WS_BORDER, 0);
        assert_eq!(next & WS_THICKFRAME, WS_THICKFRAME);
        assert_eq!(next & WS_SYSMENU, WS_SYSMENU);
        assert_eq!(next & WS_CLIPSIBLINGS, WS_CLIPSIBLINGS);
        assert_eq!(next & WS_VISIBLE, WS_VISIBLE);
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
