//! Pen facts the Win32 pointer messages carry but the windowing backend
//! drops.
//!
//! winit's Win32 backend lowers `WM_POINTER*` pen frames without three
//! things a pen stroke needs: `POINTER_FLAG_CANCELED` (a cancelled contact
//! arrives as an ordinary release), a reported pressure of zero (it becomes
//! "no pressure"), and `WM_POINTERLEAVE` (it reports a leave after every
//! lift instead, while the pen still hovers). [`PenInput`] watches the
//! window procedure ahead of winit and keeps those facts for the host, which
//! applies them to the events winit delivers for the same frame and reports
//! the real leave itself. Other platforms install nothing and report no
//! facts.

use std::sync::Arc;

use raw_window_handle::HasWindowHandle;

/// What the platform reported for the pen frame being delivered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PenFrame {
    /// Normalized pressure in `0.0..=1.0`, including a reported zero.
    /// `None` when the device reports no pressure at all.
    pub pressure: Option<f32>,
    /// The platform cancelled the contact this frame ends.
    pub canceled: bool,
}

/// Win32 pen pressure: `pressure` out of 1024 when `PEN_MASK_PRESSURE` is
/// set, otherwise none.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(crate) fn win32_pen_pressure(mask_has_pressure: bool, raw: u32) -> Option<f32> {
    mask_has_pressure.then(|| raw.min(1024) as f32 / 1024.0)
}

/// Host-owned observer of a window's pen frames.
pub struct PenInput {
    #[cfg(target_os = "windows")]
    inner: windows_hook::Hook,
}

impl PenInput {
    /// Watch `window`'s pen input. `wake` is called when a pen leaves, an
    /// event the backend does not deliver; it must only signal the host's
    /// event source, never run host callbacks inline. Other platforms ignore
    /// it.
    pub fn install_with_wake<W: HasWindowHandle + ?Sized>(
        window: &W,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        #[cfg(not(target_os = "windows"))]
        let _ = (window, wake);
        Ok(Self {
            #[cfg(target_os = "windows")]
            inner: windows_hook::Hook::install(window, wake)?,
        })
    }

    /// The pen frame the window procedure is delivering right now. `Some`
    /// only while the backend lowers that frame, which is when the host
    /// receives its events.
    pub fn current_frame(&self) -> Option<PenFrame> {
        #[cfg(target_os = "windows")]
        {
            self.inner.current_frame()
        }
        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }

    /// Whether this observer reports pen leaves, so the backend's own pen
    /// leave events are to be ignored.
    pub fn reports_leave(&self) -> bool {
        cfg!(target_os = "windows")
    }

    /// Whether a pen left the window or its sensing range since the last
    /// call.
    pub fn take_leave(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            self.inner.take_leave()
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }
}

#[cfg(target_os = "windows")]
mod windows_hook {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::sync::Arc;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::Pointer::{
        GetPointerPenInfo, GetPointerType, POINTER_FLAG_CANCELED, POINTER_PEN_INFO,
    };
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PEN_MASK_PRESSURE, POINTER_INPUT_TYPE, PT_PEN, WM_POINTERDOWN, WM_POINTERLEAVE,
        WM_POINTERUP, WM_POINTERUPDATE,
    };

    use super::PenFrame;

    const SUBCLASS_ID: usize = 0x4E_41_50_4E;

    struct HookState {
        /// Set while the backend's window procedure handles a pen frame.
        frame: Cell<Option<PenFrame>>,
        left: Cell<bool>,
        wake: Arc<dyn Fn() + Send + Sync>,
    }

    pub(super) struct Hook {
        hwnd: HWND,
        state: *mut HookState,
    }

    impl Hook {
        pub(super) fn install<W: HasWindowHandle + ?Sized>(
            window: &W,
            wake: Arc<dyn Fn() + Send + Sync>,
        ) -> Result<Self, String> {
            let handle = window
                .window_handle()
                .map_err(|error| format!("failed to acquire Win32 window handle: {error}"))?;
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return Err("pen input requires an HWND".into());
            };
            let hwnd = handle.hwnd.get() as *mut c_void;
            let state = Box::into_raw(Box::new(HookState {
                frame: Cell::new(None),
                left: Cell::new(false),
                wake,
            }));
            // SAFETY: hwnd belongs to the live winit window. `state` remains
            // allocated until the subclass is removed by `Drop`.
            let installed =
                unsafe { SetWindowSubclass(hwnd, Some(pen_proc), SUBCLASS_ID, state as usize) };
            if installed == 0 {
                // SAFETY: ownership was not transferred because install failed.
                unsafe { drop(Box::from_raw(state)) };
                return Err("SetWindowSubclass failed for pen input".into());
            }
            Ok(Self { hwnd, state })
        }

        pub(super) fn current_frame(&self) -> Option<PenFrame> {
            // SAFETY: the hook owns `state`; the window procedure that writes
            // it runs on this same thread.
            unsafe { &*self.state }.frame.get()
        }

        pub(super) fn take_leave(&self) -> bool {
            // SAFETY: as above.
            unsafe { &*self.state }.left.replace(false)
        }
    }

    impl Drop for Hook {
        fn drop(&mut self) {
            // SAFETY: dropped before the winit Window. Successful removal
            // guarantees the callback can no longer observe `state`.
            let removed = unsafe { RemoveWindowSubclass(self.hwnd, Some(pen_proc), SUBCLASS_ID) };
            if removed != 0 {
                unsafe { drop(Box::from_raw(self.state)) };
            }
        }
    }

    fn is_pen(pointer_id: u32) -> bool {
        let mut kind: POINTER_INPUT_TYPE = 0;
        // SAFETY: a query about a pointer of the message being handled.
        (unsafe { GetPointerType(pointer_id, &mut kind) } != 0) && kind == PT_PEN
    }

    /// `IS_POINTER_CANCELED_WPARAM`: the message's own pointer flags carry
    /// the cancellation even when the pen's frame can no longer be read.
    const POINTER_MESSAGE_FLAG_CANCELED: u32 = 0x8000;

    fn pen_frame(pointer_id: u32, wparam: WPARAM) -> Option<PenFrame> {
        if !is_pen(pointer_id) {
            return None;
        }
        let message_canceled = (wparam >> 16) as u32 & POINTER_MESSAGE_FLAG_CANCELED != 0;
        let mut info = POINTER_PEN_INFO::default();
        // SAFETY: as above; `info` is a writable POINTER_PEN_INFO.
        if unsafe { GetPointerPenInfo(pointer_id, &mut info) } == 0 {
            return Some(PenFrame {
                pressure: None,
                canceled: message_canceled,
            });
        }
        Some(PenFrame {
            pressure: super::win32_pen_pressure(
                info.penMask & PEN_MASK_PRESSURE != 0,
                info.pressure,
            ),
            canceled: message_canceled
                || info.pointerInfo.pointerFlags & POINTER_FLAG_CANCELED != 0,
        })
    }

    unsafe extern "system" fn pen_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _subclass_id: usize,
        ref_data: usize,
    ) -> LRESULT {
        let pointer_id = u32::from(wparam as u16);
        match message {
            WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
                let Some(frame) = pen_frame(pointer_id, wparam) else {
                    return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
                };
                // SAFETY: ref_data is the live HookState installed with this
                // subclass; the hook frees it only after removing the subclass.
                let state = unsafe { &*(ref_data as *const HookState) };
                // The backend lowers this frame inside DefSubclassProc and
                // the host receives its events there.
                let outer = state.frame.replace(Some(frame));
                let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
                state.frame.set(outer);
                result
            }
            WM_POINTERLEAVE if is_pen(pointer_id) => {
                // SAFETY: as above.
                let state = unsafe { &*(ref_data as *const HookState) };
                if !state.left.replace(true) {
                    (state.wake)();
                }
                unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
            }
            _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::win32_pen_pressure;

    #[test]
    fn a_reported_zero_is_zero_pressure_not_none() {
        assert_eq!(win32_pen_pressure(true, 0), Some(0.0));
        assert_eq!(win32_pen_pressure(true, 512), Some(0.5));
        assert_eq!(win32_pen_pressure(true, 4096), Some(1.0));
        assert_eq!(win32_pen_pressure(false, 0), None);
    }
}
