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
    use std::rc::{Rc, Weak};
    use std::sync::Arc;

    use raw_window_handle::HasWindowHandle;
    use windows_sys::Win32::Foundation::WPARAM;
    use windows_sys::Win32::UI::Input::Pointer::{
        GetPointerPenInfo, GetPointerType, POINTER_FLAG_CANCELED, POINTER_PEN_INFO,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PEN_MASK_PRESSURE, POINTER_INPUT_TYPE, PT_PEN,
    };

    use super::PenFrame;
    use crate::win32_hooks::{PenHookState, Win32WindowHooks};

    pub(super) struct Hook {
        hooks: Weak<Win32WindowHooks>,
        state: Rc<PenHookState>,
    }

    impl Hook {
        pub(super) fn install<W: HasWindowHandle + ?Sized>(
            window: &W,
            wake: Arc<dyn Fn() + Send + Sync>,
        ) -> Result<Self, String> {
            let hwnd = crate::win32::hwnd(window)
                .ok_or_else(|| "pen input requires an HWND".to_string())?;
            let hooks = Win32WindowHooks::install_or_get(hwnd)?;
            let state = Rc::new(PenHookState {
                frame: Cell::new(None),
                left: Cell::new(false),
                wake,
            });
            *hooks.pen.borrow_mut() = Some(Rc::clone(&state));
            Ok(Self {
                hooks: Rc::downgrade(&hooks),
                state,
            })
        }

        pub(super) fn current_frame(&self) -> Option<PenFrame> {
            self.state.frame.get()
        }

        pub(super) fn take_leave(&self) -> bool {
            self.state.left.replace(false)
        }
    }

    impl Drop for Hook {
        fn drop(&mut self) {
            if let Some(hooks) = self.hooks.upgrade()
                && hooks
                    .pen
                    .borrow()
                    .as_ref()
                    .is_some_and(|current| Rc::ptr_eq(current, &self.state))
            {
                hooks.clear_pen();
            }
        }
    }

    pub(crate) fn is_pen(wparam: WPARAM) -> bool {
        let pointer_id = u32::from(wparam as u16);
        let mut kind: POINTER_INPUT_TYPE = 0;
        // SAFETY: a query about a pointer of the message being handled.
        (unsafe { GetPointerType(pointer_id, &mut kind) } != 0) && kind == PT_PEN
    }

    /// `IS_POINTER_CANCELED_WPARAM`: the message's own pointer flags carry
    /// the cancellation even when the pen's frame can no longer be read.
    const POINTER_MESSAGE_FLAG_CANCELED: u32 = 0x8000;

    pub(crate) fn pen_frame(wparam: WPARAM) -> Option<PenFrame> {
        if !is_pen(wparam) {
            return None;
        }
        let pointer_id = u32::from(wparam as u16);
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
}

#[cfg(target_os = "windows")]
pub(crate) use windows_hook::{is_pen as windows_is_pen, pen_frame as windows_pen_frame};

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
