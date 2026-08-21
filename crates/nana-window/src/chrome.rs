use raw_window_handle::HasWindowHandle;

/// Prepares native titlebar dragging and client-chrome window shape.
pub fn prepare_client_chrome<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    let prepared = prepare_custom_title_bar(window);
    #[cfg(target_os = "windows")]
    let prepared = apply_rounded_corners(window) && prepared;
    prepared
}

/// Prepares native titlebar dragging for NanaUI's custom titlebar regions.
pub fn prepare_custom_title_bar<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    set_drag_enabled(window, false)
}

/// Performs one explicit NanaUI titlebar drag.
pub fn drag_custom_title_bar<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    drag(window)
}

#[cfg(target_os = "macos")]
fn set_drag_enabled<W: HasWindowHandle + ?Sized>(window: &W, enabled: bool) -> bool {
    use objc2_app_kit::NSView;
    use raw_window_handle::RawWindowHandle;

    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return false;
    };

    window.setMovable(enabled);
    true
}

#[cfg(target_os = "macos")]
fn drag<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSView};
    use raw_window_handle::RawWindowHandle;

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
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

#[cfg(not(target_os = "macos"))]
fn set_drag_enabled<W: HasWindowHandle + ?Sized>(_window: &W, _enabled: bool) -> bool {
    true
}

#[cfg(target_os = "windows")]
fn drag<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, HTCAPTION, SendMessageW, WM_NCLBUTTONDOWN,
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
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, lparam);
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
