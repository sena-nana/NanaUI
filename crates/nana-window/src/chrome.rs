use raw_window_handle::HasWindowHandle;

/// Prepares native titlebar dragging for NanaUI's custom titlebar regions.
pub fn prepare_custom_title_bar<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    set_drag_enabled(window, false)
}

/// Performs one explicit NanaUI titlebar drag.
#[cfg(target_os = "macos")]
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
