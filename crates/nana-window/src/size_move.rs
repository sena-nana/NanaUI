//! Live window-resize session. Windows posts `WM_ENTERSIZEMOVE` /
//! `WM_EXITSIZEMOVE` around the nested size-move loop.

use raw_window_handle::HasWindowHandle;

const ENTER_SIZE_MOVE: u32 = 0x0231;
const EXIT_SIZE_MOVE: u32 = 0x0232;

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
}

#[cfg(target_os = "windows")]
mod windows_hook {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE};

    const SUBCLASS_ID: usize = 0x4E_41_53_4D;

    struct HookState {
        active: AtomicBool,
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
            Ok(Self { hwnd, state })
        }

        pub(super) fn is_active(&self) -> bool {
            // SAFETY: the hook owns `state` for its full lifetime.
            unsafe { &*self.state }.active.load(Ordering::Acquire)
        }
    }

    impl Drop for Hook {
        fn drop(&mut self) {
            // SAFETY: dropped before the winit Window. Successful removal
            // guarantees the callback can no longer observe `state`.
            let removed =
                unsafe { RemoveWindowSubclass(self.hwnd, Some(size_move_proc), SUBCLASS_ID) };
            if removed != 0 {
                unsafe { drop(Box::from_raw(self.state)) };
            }
        }
    }

    unsafe extern "system" fn size_move_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _subclass_id: usize,
        ref_data: usize,
    ) -> LRESULT {
        if matches!(message, WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE) {
            // SAFETY: ref_data is the live HookState installed with this subclass.
            let state = unsafe { &*(ref_data as *const HookState) };
            let active =
                super::size_move_active_after(message, state.active.load(Ordering::Acquire));
            state.active.store(active, Ordering::Release);
            // SAFETY: hwnd is the subclassed live window.
            unsafe { InvalidateRect(hwnd, ptr::null(), 0) };
        }
        // SAFETY: forwards every message to winit's original window procedure.
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }
}

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
