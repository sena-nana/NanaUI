use raw_window_handle::HasWindowHandle;
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetCursorPos, GetWindowLongPtrW, HTCAPTION, MINMAXINFO, PostMessageW,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW,
    SetWindowLongPtrW, SetWindowPos, WM_GETMINMAXINFO, WM_NCLBUTTONDOWN,
};

use super::{
    FrameResizeEdge, client_chrome_style, dwm_border_color, dwm_corner_preference, frameless_mask,
    guarded_style_bits, hit_test_for_edge,
};
use crate::win32::WindowProp;
use crate::win32_hooks::Win32WindowHooks;

pub(super) const DWMWA_BORDER_COLOR: u32 = 34;
pub(super) const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
pub(super) const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

const DWMWA_NCRENDERING_POLICY: u32 = 2;
/// `DWMNCRP_ENABLED`.
const DWMNCRP_ENABLED: u32 = 2;
/// `DWMNCRP_DISABLED`.
const DWMNCRP_DISABLED: u32 = 1;

const fn nc_rendering_policy(enabled: bool) -> u32 {
    if enabled {
        DWMNCRP_ENABLED
    } else {
        DWMNCRP_DISABLED
    }
}

pub(crate) const fn guarded_style(style_new: isize, mask: isize) -> isize {
    guarded_style_bits(style_new, mask)
}

pub(crate) fn win32_movable_hwnd<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
    let hwnd = crate::win32::hwnd(window)?;
    (!crate::work_area::hwnd_active(hwnd)).then_some(hwnd)
}

pub(crate) fn win32_min_track_size(hwnd: HWND) -> (f64, f64) {
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

pub(super) fn drag<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    send_nc_lbutton_down(window, HTCAPTION as usize)
}

pub(super) fn resize_windows<W: HasWindowHandle + ?Sized>(
    window: &W,
    edge: FrameResizeEdge,
) -> bool {
    send_nc_lbutton_down(window, hit_test_for_edge(edge))
}

fn send_nc_lbutton_down<W: HasWindowHandle + ?Sized>(window: &W, hit: usize) -> bool {
    let Some(hwnd) = win32_movable_hwnd(window) else {
        return false;
    };
    unsafe {
        ReleaseCapture();
        let mut point = POINT { x: 0, y: 0 };
        let lparam = if GetCursorPos(&mut point) != 0 {
            ((point.y as u32) << 16 | (point.x as u32 & 0xFFFF)) as isize
        } else {
            0
        };
        // Start the native loop after the host's input callback returns. A
        // synchronous send holds winit's application handler for the entire
        // drag, so all other windows' input and redraws can only be buffered.
        PostMessageW(hwnd, WM_NCLBUTTONDOWN, hit, lparam) != 0
    }
}

/// The rounding each window last asked for, kept so the work-area
/// fullscreen can square the window and give the rounding back on leaving.
/// Stored as `rounded + 1`, since an absent property reads as zero.
static ROUNDED_PROP: WindowProp = WindowProp::new("NanaRoundedCorners");

pub(super) fn apply_window_shape<W: HasWindowHandle + ?Sized>(
    window: &W,
    rounded_corners: bool,
) -> bool {
    let Some(hwnd) = crate::win32::hwnd(window) else {
        return false;
    };
    ROUNDED_PROP.set(hwnd, usize::from(rounded_corners) + 1);
    set_dwm_shape(
        hwnd,
        rounded_corners && !crate::work_area::hwnd_active(hwnd),
    )
}

/// Re-apply the rounding the window last asked for, square while it fills
/// the work area like a fullscreen window.
pub(crate) fn reapply_window_shape(hwnd: HWND) {
    let Some(rounded) = ROUNDED_PROP.get(hwnd).checked_sub(1) else {
        return;
    };
    let fullscreen = crate::work_area::hwnd_active(hwnd);
    set_dwm_shape(hwnd, rounded == 1 && !fullscreen);
}

fn set_dwm_shape(hwnd: HWND, rounded_corners: bool) -> bool {
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

pub(super) fn set_nc_rendering_policy<W: HasWindowHandle + ?Sized>(
    window: &W,
    enabled: bool,
) -> bool {
    let Some(hwnd) = crate::win32::hwnd(window) else {
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

pub(super) fn arm_frameless_guard<W: HasWindowHandle + ?Sized>(
    window: &W,
    frameless: bool,
) -> bool {
    let Some(hwnd) = crate::win32::hwnd(window) else {
        return false;
    };
    install_style_guard(hwnd, frameless_mask(frameless))
}

pub(super) fn set_frameless_style<W: HasWindowHandle + ?Sized>(
    window: &W,
    frameless: bool,
    resizable: bool,
) -> bool {
    let Some(hwnd) = crate::win32::hwnd(window) else {
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
fn install_style_guard(hwnd: HWND, mask: isize) -> bool {
    let Ok(hooks) = Win32WindowHooks::install_or_get(hwnd) else {
        return false;
    };
    hooks.style_mask.set(mask);
    true
}

#[cfg(test)]
mod tests {
    use super::super::{FRAMELESS_STYLES, WS_CAPTION, WS_SYSMENU, WS_THICKFRAME};
    use super::install_style_guard;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_STYLE, RegisterClassW, STYLESTRUCT,
        SendMessageW, WM_STYLECHANGING, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    };

    use crate::win32::wide;

    const WS_CLIPSIBLINGS: isize = 0x0400_0000;

    /// NanaUI is a framework on somebody else's window: a native extension,
    /// an accessibility shim or an embedding host may hold its own
    /// `WM_STYLECHANGING` subclass on the same HWND. The guard must not take
    /// the message away from them, and its invariant must still be the one
    /// that survives.
    ///
    /// Runs on a real HWND because the thing under test is Win32 subclass
    /// chaining, which has no model worth asserting against.
    #[test]
    fn the_style_guard_shares_wm_stylechanging_with_another_subclass() {
        let _serial = crate::test_serial_lock();

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

        // The other subclass is installed first, so the unified hook sits above
        // it and its `DefSubclassProc` is what has to reach it.
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

        // SAFETY: destroying the window this test created; the unified hook
        // detaches itself on WM_NCDESTROY.
        unsafe {
            DestroyWindow(hwnd);
        }
    }
}
