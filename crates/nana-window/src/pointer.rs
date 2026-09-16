//! Global pointer sampling for host-owned mouse-passthrough Forward mode.
//!
//! Widgets never see HWND / NSWindow. Scene host converts the result into
//! overlay logical coordinates and runs the existing document hit test.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

/// Logical client-area position of the global pointer when it is inside
/// `window`. `None` if the pointer is outside, the query fails, or the
/// backend cannot sample while the window is not receiving pointer events.
pub fn pointer_in_client_area<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
    window: &W,
    scale_factor: f64,
    logical_size: (f32, f32),
) -> Option<(f32, f32)> {
    #[cfg(target_os = "windows")]
    {
        windows_pointer(window, finite_scale(scale_factor), logical_size)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = scale_factor;
        macos_pointer(window, logical_size)
    }
    #[cfg(target_os = "linux")]
    {
        linux_pointer(window, finite_scale(scale_factor), logical_size)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (window, scale_factor, logical_size);
        None
    }
}

#[cfg_attr(
    not(any(test, target_os = "windows", target_os = "linux")),
    allow(dead_code)
)]
fn finite_scale(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

#[cfg_attr(
    not(any(test, target_os = "windows", target_os = "linux")),
    allow(dead_code)
)]
pub(crate) fn logical_client_from_physical_offset(
    offset: (i32, i32),
    physical_size: (u32, u32),
    scale_factor: f64,
) -> Option<(f32, f32)> {
    if offset.0 < 0
        || offset.1 < 0
        || offset.0 >= physical_size.0 as i32
        || offset.1 >= physical_size.1 as i32
    {
        return None;
    }
    let scale = finite_scale(scale_factor) as f32;
    Some((offset.0 as f32 / scale, offset.1 as f32 / scale))
}

fn inside_logical(point: (f32, f32), logical_size: (f32, f32)) -> Option<(f32, f32)> {
    if point.0.is_finite()
        && point.1.is_finite()
        && point.0 >= 0.0
        && point.1 >= 0.0
        && point.0 < logical_size.0
        && point.1 < logical_size.1
    {
        Some(point)
    } else {
        None
    }
}

/// Convert an NSView-local point into top-left logical client coordinates.
///
/// winit's AppKit view is flipped (`isFlipped = true`), so `convertPoint`
/// is already top-left. An unflipped view still uses AppKit's bottom-left Y.
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
pub(crate) fn logical_from_view_point(x: f64, y: f64, height: f64, flipped: bool) -> (f32, f32) {
    let y = if flipped { y } else { height - y };
    (x as f32, y as f32)
}

#[cfg(target_os = "windows")]
fn windows_pointer<W: HasWindowHandle + ?Sized>(
    window: &W,
    scale_factor: f64,
    logical_size: (f32, f32),
) -> Option<(f32, f32)> {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetCursorPos};

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    let hwnd = handle.hwnd.get() as HWND;
    let mut mouse = POINT::default();
    let mut client = RECT::default();
    unsafe {
        if GetCursorPos(&mut mouse) == 0 {
            return None;
        }
        if ScreenToClient(hwnd, &mut mouse) == 0 {
            return None;
        }
        if GetClientRect(hwnd, &mut client) == 0 {
            return None;
        }
    }
    let physical = (
        (client.right - client.left).max(0) as u32,
        (client.bottom - client.top).max(0) as u32,
    );
    let logical = logical_client_from_physical_offset((mouse.x, mouse.y), physical, scale_factor)?;
    inside_logical(logical, logical_size)
}

#[cfg(target_os = "macos")]
fn macos_pointer<W: HasWindowHandle + ?Sized>(
    window: &W,
    logical_size: (f32, f32),
) -> Option<(f32, f32)> {
    use objc2_app_kit::NSView;
    use raw_window_handle::RawWindowHandle;

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: ns_view is a live NSView owned by this window.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let ns_window = view.window()?;
    let location = ns_window.mouseLocationOutsideOfEventStream();
    let in_view = view.convertPoint_fromView(location, None);
    let bounds = view.bounds();
    let height = if bounds.size.height > 0.0 {
        bounds.size.height
    } else {
        f64::from(logical_size.1)
    };
    inside_logical(
        logical_from_view_point(in_view.x, in_view.y, height, view.isFlipped()),
        logical_size,
    )
}

#[cfg(target_os = "linux")]
fn linux_pointer<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
    window: &W,
    scale_factor: f64,
    logical_size: (f32, f32),
) -> Option<(f32, f32)> {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
    use std::os::raw::{c_int, c_uint, c_ulong};

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Xlib(handle) = handle.as_raw() else {
        // Wayland cannot query the global pointer while the overlay is not
        // receiving events. Do not invent a position from a stale cursor.
        return None;
    };
    // raw-window-handle 0.6 keeps the X display on the *display* handle;
    // `XlibWindowHandle` is only the window id and its visual.
    let display = window.display_handle().ok()?;
    let RawDisplayHandle::Xlib(display) = display.as_raw() else {
        return None;
    };
    let display = display.display?;
    let xlib = x11_dl::xlib::Xlib::open().ok()?;
    let mut root = 0 as c_ulong;
    let mut child = 0 as c_ulong;
    let mut root_x = 0 as c_int;
    let mut root_y = 0 as c_int;
    let mut win_x = 0 as c_int;
    let mut win_y = 0 as c_int;
    let mut mask = 0 as c_uint;
    let queried = unsafe {
        (xlib.XQueryPointer)(
            display.as_ptr().cast(),
            handle.window as c_ulong,
            &mut root,
            &mut child,
            &mut root_x,
            &mut root_y,
            &mut win_x,
            &mut win_y,
            &mut mask,
        )
    };
    if queried == 0 {
        return None;
    }
    let physical = (
        (logical_size.0 as f64 * scale_factor).round().max(1.0) as u32,
        (logical_size.1 as f64 * scale_factor).round().max(1.0) as u32,
    );
    let logical = logical_client_from_physical_offset((win_x, win_y), physical, scale_factor)?;
    inside_logical(logical, logical_size)
}

#[cfg(test)]
mod tests {
    use super::{logical_client_from_physical_offset, logical_from_view_point};

    #[test]
    fn physical_offset_maps_to_logical_client_and_rejects_the_exterior() {
        assert_eq!(
            logical_client_from_physical_offset((100, 40), (200, 100), 2.0),
            Some((50.0, 20.0))
        );
        assert_eq!(
            logical_client_from_physical_offset((-1, 0), (200, 100), 2.0),
            None
        );
        assert_eq!(
            logical_client_from_physical_offset((200, 0), (200, 100), 2.0),
            None
        );
        assert_eq!(
            logical_client_from_physical_offset((0, 100), (200, 100), 2.0),
            None
        );
    }

    #[test]
    fn flipped_nsview_keeps_top_left_y() {
        assert_eq!(
            logical_from_view_point(10.0, 20.0, 100.0, true),
            (10.0, 20.0)
        );
        assert_eq!(
            logical_from_view_point(10.0, 20.0, 100.0, false),
            (10.0, 80.0)
        );
    }
}
