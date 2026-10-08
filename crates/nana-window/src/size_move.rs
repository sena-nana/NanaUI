//! Live window-resize session. Windows posts `WM_ENTERSIZEMOVE` /
//! `WM_EXITSIZEMOVE` around the nested size-move loop, and `WM_SIZING` for
//! every step of a user resize inside it.

use raw_window_handle::HasWindowHandle;

#[cfg(any(target_os = "windows", test))]
const ENTER_SIZE_MOVE: u32 = 0x0231;
#[cfg(any(target_os = "windows", test))]
const EXIT_SIZE_MOVE: u32 = 0x0232;

#[cfg(any(target_os = "windows", test))]
pub(crate) fn size_move_active_after(message: u32, was_active: bool) -> bool {
    match message {
        ENTER_SIZE_MOVE => true,
        EXIT_SIZE_MOVE => false,
        _ => was_active,
    }
}

/// Host-owned observer for the platform live-resize session.
pub struct LiveSizeMove {
    #[cfg(target_os = "windows")]
    inner: windows_hook::Hook,
}

impl LiveSizeMove {
    pub fn install<W: HasWindowHandle + ?Sized>(window: &W) -> Result<Self, String> {
        #[cfg(not(target_os = "windows"))]
        let _ = window;
        Ok(Self {
            #[cfg(target_os = "windows")]
            inner: windows_hook::Hook::install(window)?,
        })
    }

    pub fn is_active(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            self.inner.is_active()
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }

    /// Keep the client area at `ratio` (width over height) while the user
    /// drags a frame edge, never below `minimum` (logical client size) on
    /// either edge. `None` releases the lock. Covers both the platform's own
    /// size-move loop and [`crate::LiveFrameResize`]; a maximized window is
    /// left alone. Programmatic resizes are the host's to conform.
    ///
    /// Windows only; other platforms ignore it.
    pub fn set_content_aspect_ratio(&self, ratio: Option<f64>, minimum: (f64, f64)) {
        let lock = ratio
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
            .map(|ratio| crate::aspect::AspectLock {
                ratio,
                minimum: (minimum.0.max(0.0), minimum.1.max(0.0)),
            });
        #[cfg(target_os = "windows")]
        self.inner.set_aspect(lock);
        #[cfg(not(target_os = "windows"))]
        let _ = lock;
    }
}

#[cfg(target_os = "windows")]
mod windows_hook {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetPropW, GetWindowRect, IsZoomed, RemovePropW, SetPropW, WM_ENTERSIZEMOVE,
        WM_EXITSIZEMOVE, WM_SETTINGCHANGE, WM_SIZING,
    };

    use crate::aspect::AspectLock;

    const SUBCLASS_ID: usize = 0x4E_41_53_4D;

    /// Holds the installed `HookState` for readers outside the window
    /// procedure. Replaces `GetWindowSubclass`, which comctl32 v5.82 (the
    /// default without a Common-Controls v6 manifest) exports only by ordinal:
    /// a by-name import stops every hosted binary from loading.
    fn state_prop() -> Vec<u16> {
        "NanaUI.SizeMove"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    struct HookState {
        active: AtomicBool,
        /// Logical minimum; scaled to the window's DPI at each use.
        aspect: Cell<Option<AspectLock>>,
        /// Client size when the current size-move loop began.
        reference: Cell<Option<(f64, f64)>>,
    }

    pub(super) struct Hook {
        hwnd: HWND,
        state: *mut HookState,
    }

    impl Hook {
        pub(super) fn install<W: HasWindowHandle + ?Sized>(window: &W) -> Result<Self, String> {
            let handle = window
                .window_handle()
                .map_err(|error| format!("failed to acquire Win32 window handle: {error}"))?;
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return Err("live size-move requires an HWND".into());
            };
            let hwnd = handle.hwnd.get() as *mut c_void;
            let state = Box::into_raw(Box::new(HookState {
                active: AtomicBool::new(false),
                aspect: Cell::new(None),
                reference: Cell::new(None),
            }));
            // SAFETY: hwnd belongs to the live winit window. `state` remains
            // allocated until the subclass is removed by `Drop`.
            let installed = unsafe {
                SetWindowSubclass(hwnd, Some(size_move_proc), SUBCLASS_ID, state as usize)
            };
            if installed == 0 {
                // SAFETY: ownership was not transferred because install failed.
                unsafe { drop(Box::from_raw(state)) };
                return Err("SetWindowSubclass failed for live size-move".into());
            }
            let prop = state_prop();
            // SAFETY: hwnd is live; the property is removed before `state` is freed.
            if unsafe { SetPropW(hwnd, prop.as_ptr(), state as *mut c_void) } == 0 {
                // SAFETY: the subclass was just installed with this state.
                if unsafe { RemoveWindowSubclass(hwnd, Some(size_move_proc), SUBCLASS_ID) } != 0 {
                    unsafe { drop(Box::from_raw(state)) };
                }
                return Err("SetPropW failed for live size-move".into());
            }
            Ok(Self { hwnd, state })
        }

        pub(super) fn is_active(&self) -> bool {
            // SAFETY: the hook owns `state` for its full lifetime.
            unsafe { &*self.state }.active.load(Ordering::Acquire)
        }

        pub(super) fn set_aspect(&self, lock: Option<AspectLock>) {
            // SAFETY: the hook owns `state`; the window procedure that also
            // reads it runs on this same thread.
            unsafe { &*self.state }.aspect.set(lock);
        }
    }

    impl Drop for Hook {
        fn drop(&mut self) {
            let prop = state_prop();
            // SAFETY: readers of the property stop seeing `state` before it is freed.
            unsafe { RemovePropW(self.hwnd, prop.as_ptr()) };
            // SAFETY: dropped before the winit Window. Successful removal
            // guarantees the callback can no longer observe `state`.
            let removed =
                unsafe { RemoveWindowSubclass(self.hwnd, Some(size_move_proc), SUBCLASS_ID) };
            if removed != 0 {
                unsafe { drop(Box::from_raw(self.state)) };
            }
        }
    }

    /// The aspect lock installed on `hwnd`, with its minimum in physical
    /// pixels. `None` for a window without the hook or without a lock, and
    /// for a maximized window, which keeps whatever the platform gave it.
    pub(crate) fn aspect_lock(hwnd: HWND) -> Option<AspectLock> {
        let prop = state_prop();
        // SAFETY: GetPropW only reads the property list of hwnd.
        let data = unsafe { GetPropW(hwnd, prop.as_ptr()) } as *const HookState;
        if data.is_null() {
            return None;
        }
        // SAFETY: the property holds the live HookState until `Hook::drop`
        // removes it, before the state is freed.
        let state = unsafe { &*data };
        physical_lock(hwnd, state.aspect.get()?)
    }

    fn physical_lock(hwnd: HWND, lock: AspectLock) -> Option<AspectLock> {
        // SAFETY: plain queries on a live window.
        if unsafe { IsZoomed(hwnd) } != 0 {
            return None;
        }
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };
        Some(AspectLock {
            ratio: lock.ratio,
            minimum: (lock.minimum.0 * scale, lock.minimum.1 * scale),
        })
    }

    /// Outer minus client size per axis, and the client size.
    pub(crate) fn frame_and_client(hwnd: HWND) -> Option<((f64, f64), (f64, f64))> {
        let (mut outer, mut client) = (RECT::default(), RECT::default());
        // SAFETY: plain queries on a live window.
        if unsafe { GetWindowRect(hwnd, &mut outer) } == 0
            || unsafe { GetClientRect(hwnd, &mut client) } == 0
        {
            return None;
        }
        let client_size = (
            f64::from(client.right - client.left),
            f64::from(client.bottom - client.top),
        );
        let frame = (
            f64::from(outer.right - outer.left) - client_size.0,
            f64::from(outer.bottom - outer.top) - client_size.1,
        );
        Some((frame, client_size))
    }

    /// Bring a `WM_SIZING` drag rectangle to the locked ratio. Answers
    /// whether it did.
    fn constrain_sizing(hwnd: HWND, state: &HookState, wparam: WPARAM, lparam: LPARAM) -> bool {
        let Some(lock) = state.aspect.get() else {
            return false;
        };
        let Some(edge) = crate::aspect::sizing_edge(wparam) else {
            return false;
        };
        let Some(lock) = physical_lock(hwnd, lock) else {
            return false;
        };
        let Some((frame, client)) = frame_and_client(hwnd) else {
            return false;
        };
        let reference = state.reference.get().unwrap_or(client);
        // SAFETY: WM_SIZING's lParam is the drag rectangle, writable.
        let rect = unsafe { &mut *(lparam as *mut RECT) };
        let next = lock.constrain_frame(
            [
                f64::from(rect.left),
                f64::from(rect.top),
                f64::from(rect.right),
                f64::from(rect.bottom),
            ],
            edge,
            frame,
            reference,
        );
        *rect = RECT {
            left: next[0] as i32,
            top: next[1] as i32,
            right: next[2] as i32,
            bottom: next[3] as i32,
        };
        true
    }

    unsafe extern "system" fn size_move_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _subclass_id: usize,
        ref_data: usize,
    ) -> LRESULT {
        // Every host window carries this subclass, so it also observes the
        // system setting broadcasts NanaUI reports to programs.
        if message == WM_SETTINGCHANGE {
            crate::motion_preference::observe_setting_change(wparam);
            crate::contrast_preference::observe_setting_change(wparam);
        }
        if matches!(message, WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE) {
            // SAFETY: ref_data is the live HookState installed with this subclass.
            let state = unsafe { &*(ref_data as *const HookState) };
            let active =
                super::size_move_active_after(message, state.active.load(Ordering::Acquire));
            state.active.store(active, Ordering::Release);
            let reference = if active {
                frame_and_client(hwnd).map(|(_, client)| client)
            } else {
                None
            };
            state.reference.set(reference);
            // SAFETY: hwnd is the subclassed live window.
            unsafe { InvalidateRect(hwnd, ptr::null(), 0) };
        }
        if message == WM_SIZING {
            // winit snaps to resize increments first; the ratio has the last
            // word, because Windows applies the rectangle as it comes back.
            // SAFETY: forwards to winit's original window procedure.
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            // SAFETY: ref_data is the live HookState installed with this subclass.
            let state = unsafe { &*(ref_data as *const HookState) };
            return if constrain_sizing(hwnd, state, wparam, lparam) {
                1
            } else {
                result
            };
        }
        // SAFETY: forwards every message to winit's original window procedure.
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows_hook::{aspect_lock, frame_and_client};

#[cfg(test)]
mod tests {
    use super::{ENTER_SIZE_MOVE, EXIT_SIZE_MOVE, size_move_active_after};

    #[test]
    fn enter_and_exit_messages_toggle_the_live_session() {
        assert!(size_move_active_after(ENTER_SIZE_MOVE, false));
        assert!(!size_move_active_after(EXIT_SIZE_MOVE, true));
        assert!(size_move_active_after(0x0005, true));
        assert!(!size_move_active_after(0x0005, false));
    }
}
