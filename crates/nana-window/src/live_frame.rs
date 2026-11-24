#[cfg(any(target_os = "macos", target_os = "windows"))]
use raw_window_handle::HasWindowHandle;

use crate::FrameResizeEdge;

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
    /// The window's content aspect lock and its frame as the resize began.
    #[cfg(target_os = "windows")]
    aspect: Option<(crate::aspect::AspectLock, crate::aspect::SizingBasis)>,
}

#[cfg(target_os = "macos")]
impl LiveFrameResize {
    pub fn begin<W: HasWindowHandle + ?Sized>(window: &W, edge: FrameResizeEdge) -> Option<Self> {
        let window = crate::chrome::appkit_movable_window(window)?;
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
        let Some(window) = crate::chrome::appkit_movable_window(window) else {
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
        let hwnd = crate::chrome::win32_movable_hwnd(window)?;
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
        let min = crate::chrome::win32_min_track_size(hwnd);
        let aspect =
            crate::size_move::aspect_lock(hwnd).zip(crate::size_move::sizing_basis(hwnd, min));
        Some(Self {
            edge,
            origin_x: f64::from(rect.left),
            origin_y: f64::from(rect.top),
            width: f64::from(rect.right - rect.left),
            height: f64::from(rect.bottom - rect.top),
            mouse_x: f64::from(mouse.x),
            mouse_y: f64::from(mouse.y),
            min,
            aspect,
        })
    }

    pub fn update<W: HasWindowHandle + ?Sized>(&self, window: &W) -> bool {
        let Some(hwnd) = crate::chrome::win32_movable_hwnd(window) else {
            return false;
        };
        let mut mouse = windows_sys::Win32::Foundation::POINT::default();
        if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut mouse) } == 0 {
            return false;
        }
        let mut next = live_frame_after_delta(
            [self.origin_x, self.origin_y, self.width, self.height],
            f64::from(mouse.x) - self.mouse_x,
            f64::from(mouse.y) - self.mouse_y,
            self.edge,
            self.min,
            true,
        );
        if let Some((lock, basis)) = self.aspect {
            next = lock.constrain_origin_size(next, self.edge, basis);
        }
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
/// exists beside [`crate::drag_custom_title_bar`]: that one hands the gesture to
/// the platform, which only understands a held primary button. AppKit ignores a
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

/// Where the cursor holds a window: the fraction of the frame's width to its
/// left, and its distance below the frame's top edge (in the platform's
/// screen units). Taken before a maximized window is restored, so the
/// restored frame can be put back under the cursor the way the system move
/// loop does.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGrab {
    across: f64,
    down: f64,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl FrameGrab {
    /// The grab for a frame `(left, top, width)` held at `(x, y)`, in a
    /// top-down coordinate space.
    fn at(left: f64, top: f64, width: f64, x: f64, y: f64) -> Self {
        Self {
            across: if width > 0.0 {
                ((x - left) / width).clamp(0.0, 1.0)
            } else {
                0.5
            },
            down: (y - top).max(0.0),
        }
    }

    /// The top-left, top-down origin that puts a `width` x `height` frame
    /// under `(x, y)` at this grab. The vertical offset keeps the cursor
    /// inside the frame when the restored frame is shorter.
    fn origin(self, width: f64, height: f64, x: f64, y: f64) -> (f64, f64) {
        let down = self.down.min((height - 1.0).max(0.0));
        (x - self.across * width, y - down)
    }
}

#[cfg(target_os = "macos")]
impl LiveFrameMove {
    /// Where the cursor holds `window` now. AppKit screen space is bottom-up,
    /// so it is flipped here and the grab is top-down like on Windows.
    pub fn grab<W: HasWindowHandle + ?Sized>(window: &W) -> Option<FrameGrab> {
        let frame = crate::chrome::appkit_movable_window(window)?.frame();
        let mouse = objc2_app_kit::NSEvent::mouseLocation();
        let top = frame.origin.y + frame.size.height;
        Some(FrameGrab::at(
            frame.origin.x,
            -top,
            frame.size.width,
            mouse.x,
            -mouse.y,
        ))
    }

    /// Starts a move with the (just restored) frame placed under the cursor
    /// at `grab`.
    pub fn begin_at<W: HasWindowHandle + ?Sized>(window: &W, grab: FrameGrab) -> Option<Self> {
        let appkit = crate::chrome::appkit_movable_window(window)?;
        let size = appkit.frame().size;
        let mouse = objc2_app_kit::NSEvent::mouseLocation();
        let (left, top) = grab.origin(size.width, size.height, mouse.x, -mouse.y);
        let origin_y = -top - size.height;
        appkit.setFrameOrigin(objc2_foundation::NSPoint::new(left, origin_y));
        Some(Self {
            origin_x: left,
            origin_y,
            mouse_x: mouse.x,
            mouse_y: mouse.y,
        })
    }

    pub fn begin<W: HasWindowHandle + ?Sized>(window: &W) -> Option<Self> {
        let window = crate::chrome::appkit_movable_window(window)?;
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
        let Some(window) = crate::chrome::appkit_movable_window(window) else {
            return false;
        };
        window.setFrameOrigin(objc2_foundation::NSPoint::new(x, y));
        true
    }
}

#[cfg(target_os = "windows")]
impl LiveFrameMove {
    fn window_rect_and_cursor(
        hwnd: windows_sys::Win32::Foundation::HWND,
    ) -> Option<(
        windows_sys::Win32::Foundation::RECT,
        windows_sys::Win32::Foundation::POINT,
    )> {
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        let mut mouse = windows_sys::Win32::Foundation::POINT::default();
        unsafe {
            if windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) == 0
                || windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut mouse) == 0
            {
                return None;
            }
        }
        Some((rect, mouse))
    }

    /// Where the cursor holds `window` now, in screen pixels.
    pub fn grab<W: HasWindowHandle + ?Sized>(window: &W) -> Option<FrameGrab> {
        let (rect, mouse) =
            Self::window_rect_and_cursor(crate::chrome::win32_movable_hwnd(window)?)?;
        Some(FrameGrab::at(
            f64::from(rect.left),
            f64::from(rect.top),
            f64::from(rect.right - rect.left),
            f64::from(mouse.x),
            f64::from(mouse.y),
        ))
    }

    /// Starts a move with the (just restored) frame placed under the cursor
    /// at `grab`.
    pub fn begin_at<W: HasWindowHandle + ?Sized>(window: &W, grab: FrameGrab) -> Option<Self> {
        let (rect, mouse) =
            Self::window_rect_and_cursor(crate::chrome::win32_movable_hwnd(window)?)?;
        let (left, top) = grab.origin(
            f64::from(rect.right - rect.left),
            f64::from(rect.bottom - rect.top),
            f64::from(mouse.x),
            f64::from(mouse.y),
        );
        let live = Self {
            origin_x: left.round(),
            origin_y: top.round(),
            mouse_x: f64::from(mouse.x),
            mouse_y: f64::from(mouse.y),
        };
        live.set_origin(window, live.origin_x, live.origin_y);
        Some(live)
    }

    pub fn begin<W: HasWindowHandle + ?Sized>(window: &W) -> Option<Self> {
        let (rect, mouse) =
            Self::window_rect_and_cursor(crate::chrome::win32_movable_hwnd(window)?)?;
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
        let Some(hwnd) = crate::chrome::win32_movable_hwnd(window) else {
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

// Used by the macOS and Windows live-resize paths; on other targets only the
// tests reach it.
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

#[cfg(test)]
mod tests {
    use super::{FrameResizeEdge, live_frame_after_delta};

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn a_restored_frame_comes_back_under_the_cursor_at_the_same_share_of_its_width() {
        // Maximized 1920 wide, held three quarters across and 12 below the top.
        let grab = super::FrameGrab::at(0.0, 0.0, 1920.0, 1440.0, 12.0);
        // Restored to 800 x 600: the cursor stays three quarters across.
        assert_eq!(grab.origin(800.0, 600.0, 1440.0, 12.0), (840.0, 0.0));
        // A grab lower than the restored frame is tall keeps the cursor inside.
        let deep = super::FrameGrab::at(0.0, 0.0, 1920.0, 100.0, 900.0);
        assert_eq!(deep.origin(800.0, 600.0, 100.0, 900.0).1, 301.0);
        // A degenerate width holds the middle.
        let empty = super::FrameGrab::at(0.0, 0.0, 0.0, 5.0, 5.0);
        assert_eq!(empty.origin(200.0, 100.0, 5.0, 5.0).0, -95.0);
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
    }
}
