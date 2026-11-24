//! Windows fullscreen that leaves the taskbar usable.
//!
//! A borderless window the size of its monitor is fullscreen to the shell:
//! winit marks it through `ITaskbarList2::MarkFullscreenWindow`, and Explorer
//! drops the taskbar beneath any active window covering a whole monitor. A
//! transparent window then shows the taskbar through itself while taking every
//! click meant for it. This mode fills the monitor's work area instead, and
//! leaves one pixel to an auto-hide taskbar so it can still slide in. The
//! window keeps its frameless style; the placement before entering is restored
//! on leaving, maximized included.

use raw_window_handle::HasWindowHandle;

/// Fill the work area of the monitor holding `display_origin` (a physical
/// desktop point), or of the window's own monitor. Already in the mode, the
/// window refits to that monitor. Answers whether the window is in the mode;
/// other platforms always answer false.
pub fn enter_work_area_fullscreen<W: HasWindowHandle + ?Sized>(
    window: &W,
    display_origin: Option<(i32, i32)>,
) -> bool {
    #[cfg(target_os = "windows")]
    {
        imp::hwnd(window).is_some_and(|hwnd| imp::enter(hwnd, display_origin))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, display_origin);
        false
    }
}

/// Leave the mode and restore the placement from before it. Answers whether
/// the window was in the mode.
pub fn leave_work_area_fullscreen<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    #[cfg(target_os = "windows")]
    {
        imp::hwnd(window).is_some_and(imp::leave)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        false
    }
}

/// Whether the window is in the work-area fullscreen.
pub fn work_area_fullscreen<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    #[cfg(target_os = "windows")]
    {
        imp::hwnd(window).is_some_and(imp::active)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        false
    }
}

/// The rectangle the window fills: the work area, less one pixel at each
/// monitor edge an auto-hide taskbar lives on when the work area is the whole
/// monitor. A window covering the monitor would keep that taskbar hidden.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn fill_rect(
    monitor: (i32, i32, i32, i32),
    work: (i32, i32, i32, i32),
    autohide: [bool; 4],
) -> (i32, i32, i32, i32) {
    if work != monitor {
        return work;
    }
    let (mut left, mut top, mut right, mut bottom) = work;
    let [on_left, on_top, on_right, on_bottom] = autohide;
    left += i32::from(on_left);
    top += i32::from(on_top);
    right -= i32::from(on_right);
    bottom -= i32::from(on_bottom);
    (left, top, right, bottom)
}

#[cfg(target_os = "windows")]
pub(crate) use imp::active as hwnd_active;
#[cfg(target_os = "windows")]
pub(crate) use imp::{fill_maximized, refit_active};

#[cfg(target_os = "windows")]
mod imp {
    use raw_window_handle::HasWindowHandle;
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
        MonitorFromWindow,
    };
    use windows_sys::Win32::UI::Shell::{
        ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETAUTOHIDEBAREX, APPBARDATA, SHAppBarMessage,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, IsIconic, IsZoomed, MINMAXINFO, SW_RESTORE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPlacement, SetWindowPos, ShowWindow,
        WINDOWPLACEMENT,
    };

    use crate::win32_hooks::Win32WindowHooks;

    pub(super) fn hwnd<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
        crate::win32::hwnd(window)
    }

    pub(crate) fn active(hwnd: HWND) -> bool {
        Win32WindowHooks::get(hwnd).is_some_and(|hooks| hooks.work_area_placement.get().is_some())
    }

    pub(super) fn enter(hwnd: HWND, display_origin: Option<(i32, i32)>) -> bool {
        // SAFETY: hwnd is the caller's live window on this thread.
        let monitor = unsafe {
            match display_origin {
                Some((x, y)) => MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST),
                None => MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            }
        };
        if active(hwnd) {
            return fit(hwnd, monitor);
        }
        let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
        placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
        // SAFETY: placement is a correctly sized out parameter.
        if unsafe { GetWindowPlacement(hwnd, &mut placement) } == 0 {
            return false;
        }
        let Ok(hooks) = Win32WindowHooks::install_or_get(hwnd) else {
            return false;
        };
        hooks.work_area_placement.set(Some(placement));
        // A maximized window keeps its maximized frame through SetWindowPos.
        // SAFETY: plain state queries and a show command on the live window.
        unsafe {
            if IsZoomed(hwnd) != 0 || IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
        }
        crate::chrome::reapply_window_shape(hwnd);
        if fit(hwnd, monitor) {
            true
        } else {
            leave(hwnd);
            false
        }
    }

    pub(super) fn leave(hwnd: HWND) -> bool {
        let Some(hooks) = Win32WindowHooks::get(hwnd) else {
            return false;
        };
        let Some(placement) = hooks.work_area_placement.take() else {
            return false;
        };
        crate::chrome::reapply_window_shape(hwnd);
        // SAFETY: placement came from GetWindowPlacement on this window.
        unsafe { SetWindowPlacement(hwnd, &placement) };
        true
    }

    pub(crate) fn refit_active(hwnd: HWND) {
        if active(hwnd) {
            // SAFETY: hwnd is the live window this procedure serves.
            let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
            fit(hwnd, monitor);
        }
    }

    /// The monitor's rectangle and the one a window filling it may take:
    /// `fill_rect` of its work area.
    fn monitor_fill(monitor: HMONITOR) -> Option<(RECT, (i32, i32, i32, i32))> {
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        // SAFETY: info is a correctly sized out parameter.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return None;
        }
        let tuple = |rect: RECT| (rect.left, rect.top, rect.right, rect.bottom);
        let autohide = [ABE_LEFT, ABE_TOP, ABE_RIGHT, ABE_BOTTOM]
            .map(|edge| autohide_bar(edge, info.rcMonitor));
        Some((
            info.rcMonitor,
            super::fill_rect(tuple(info.rcMonitor), tuple(info.rcWork), autohide),
        ))
    }

    /// Answers `WM_GETMINMAXINFO` for a window without a caption: maximized,
    /// it fills the work area of the monitor it is on, like a captioned
    /// window does, instead of the whole monitor. Without a caption Windows
    /// maximizes to the monitor, the shell takes the window for a fullscreen
    /// one and sinks the taskbar beneath it; winit then only insets the
    /// client, so the strip over the taskbar still belongs to the window and
    /// takes its clicks. The position is relative to that monitor's origin.
    ///
    /// # Safety
    /// `info` is the `MINMAXINFO` a `WM_GETMINMAXINFO` for `hwnd` passed.
    pub(crate) unsafe fn fill_maximized(hwnd: HWND, info: *mut MINMAXINFO) {
        if info.is_null() {
            return;
        }
        // SAFETY: hwnd is the live window whose message is being answered.
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let Some((rect, (left, top, right, bottom))) = monitor_fill(monitor) else {
            return;
        };
        // SAFETY: the sender owns `info` for the duration of the message.
        let info = unsafe { &mut *info };
        info.ptMaxPosition = POINT {
            x: left - rect.left,
            y: top - rect.top,
        };
        info.ptMaxSize = POINT {
            x: right - left,
            y: bottom - top,
        };
    }

    fn fit(hwnd: HWND, monitor: HMONITOR) -> bool {
        let Some((_, (left, top, right, bottom))) = monitor_fill(monitor) else {
            return false;
        };
        // SAFETY: a geometry change on the live window, z-order untouched.
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                left,
                top,
                right - left,
                bottom - top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            ) != 0
        }
    }

    fn autohide_bar(edge: u32, monitor: RECT) -> bool {
        let mut data: APPBARDATA = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<APPBARDATA>() as u32;
        data.uEdge = edge;
        data.rc = monitor;
        // SAFETY: data is a correctly sized in/out parameter.
        unsafe { SHAppBarMessage(ABM_GETAUTOHIDEBAREX, &mut data) != 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::fill_rect;

    const MONITOR: (i32, i32, i32, i32) = (0, 0, 1920, 1080);

    #[test]
    fn a_docked_taskbar_keeps_its_strip() {
        let work = (0, 0, 1920, 1032);
        assert_eq!(fill_rect(MONITOR, work, [false, false, false, true]), work);
    }

    #[test]
    fn an_auto_hide_taskbar_keeps_one_pixel_on_its_edge() {
        assert_eq!(
            fill_rect(MONITOR, MONITOR, [false, false, false, true]),
            (0, 0, 1920, 1079)
        );
        assert_eq!(
            fill_rect(MONITOR, MONITOR, [true, true, false, false]),
            (1, 1, 1920, 1080)
        );
    }

    #[test]
    fn without_a_taskbar_the_whole_monitor_is_filled() {
        assert_eq!(fill_rect(MONITOR, MONITOR, [false; 4]), MONITOR);
    }
}
