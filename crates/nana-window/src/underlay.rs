//! Underlay windows: a window glued directly beneath another one.
//!
//! An underlay covers exactly its parent's client area, sits directly below
//! it in z-order (in the parent's band, so a topmost parent keeps a topmost
//! underlay), moves and resizes with it, hides while the parent is hidden or
//! minimized, and never takes input or activation: every click lands on the
//! parent above it. Unlike the shadow companion it is an ordinary top-level
//! window with its own surface and title, so screen-capture software can list
//! and capture it on its own — the point of splitting one visual window into
//! two native ones.
//!
//! The parent's frame is the only authority; the underlay never persists or
//! constrains its own geometry. The host calls [`Underlay::sync`] on every
//! geometry, mode, focus or visibility change of the parent — not per frame:
//! on macOS it re-orders the window, which is a window-server round trip. On
//! Windows a subclass on the parent already follows it within the same
//! message, and there `sync` returns early when nothing moved.

use raw_window_handle::HasWindowHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnderlayError {
    /// This platform has no underlay windows.
    Unsupported(String),
    /// The platform refused to attach the two windows.
    Failed(String),
}

impl std::fmt::Display for UnderlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(f, "underlay unsupported: {reason}"),
            Self::Failed(reason) => write!(f, "underlay failed: {reason}"),
        }
    }
}

impl std::error::Error for UnderlayError {}

/// The binding of an underlay window to its parent. Dropping it detaches the
/// two; drop it before either window is destroyed.
pub struct Underlay {
    inner: platform::Underlay,
}

impl Underlay {
    /// Glue `underlay` beneath `parent`. Both must be live top-level windows
    /// of this process, on the main thread.
    pub fn attach<P, U>(parent: &P, underlay: &U) -> Result<Self, UnderlayError>
    where
        P: HasWindowHandle + ?Sized,
        U: HasWindowHandle + ?Sized,
    {
        platform::Underlay::attach(parent, underlay).map(|inner| Self { inner })
    }

    /// Put the underlay over the parent's client area, directly below it,
    /// shown exactly while the parent is. Does nothing when already there.
    pub fn sync(&self) {
        self.inner.sync();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindow, NSWindowCollectionBehavior, NSWindowOrderingMode};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::UnderlayError;

    fn ns_window<W: HasWindowHandle + ?Sized>(window: &W) -> Option<Retained<NSWindow>> {
        MainThreadMarker::new()?;
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: the AppKit handle's ns_view is a live NSView owned by the window.
        let view: Retained<NSView> =
            unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>())? };
        view.window()
    }

    pub(super) struct Underlay {
        parent: Retained<NSWindow>,
        window: Retained<NSWindow>,
    }

    impl Underlay {
        pub(super) fn attach<P, U>(parent: &P, underlay: &U) -> Result<Self, UnderlayError>
        where
            P: HasWindowHandle + ?Sized,
            U: HasWindowHandle + ?Sized,
        {
            let failed = |reason: &str| UnderlayError::Failed(reason.into());
            let parent = ns_window(parent).ok_or_else(|| failed("parent has no NSWindow"))?;
            let window = ns_window(underlay).ok_or_else(|| failed("underlay has no NSWindow"))?;
            // A window that never set it hit-tests by alpha, so a click on a
            // transparent pixel of the parent would fall through to the
            // underlay's content — or past it to the desktop.
            parent.setIgnoresMouseEvents(false);
            window.setIgnoresMouseEvents(true);
            window.setHasShadow(false);
            // Out of the window cycle; allowed into the parent's fullscreen
            // Space.
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::IgnoresCycle
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            let this = Self { parent, window };
            this.sync();
            Ok(this)
        }

        pub(super) fn sync(&self) {
            let parent = &self.parent;
            let window = &self.window;
            if !parent.isVisible() || parent.isMiniaturized() {
                if window.isVisible() {
                    window.orderOut(None);
                }
                return;
            }
            let Some(content) = parent.contentView() else {
                return;
            };
            let client = parent.convertRectToScreen(content.frame());
            if window.frame() != client {
                window.setFrame_display(client, false);
            }
            if window.level() != parent.level() {
                window.setLevel(parent.level());
            }
            // Ordered, not attached: AppKit captures a window together with
            // its child windows, so a child underlay would bring the parent's
            // controls into every capture of it.
            window.orderWindow_relativeTo(NSWindowOrderingMode::Below, parent.windowNumber());
        }
    }

    impl Drop for Underlay {
        fn drop(&mut self) {
            self.window.orderOut(None);
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GW_HWNDNEXT, GWL_EXSTYLE, GetClientRect, GetWindow, GetWindowRect, HTTRANSPARENT, IsIconic,
        IsWindowVisible, MA_NOACTIVATE, STYLESTRUCT, SWP_NOACTIVATE, SWP_NOOWNERZORDER,
        SWP_NOZORDER, SetWindowPos, WINDOWPOS, WM_MOUSEACTIVATE, WM_NCDESTROY, WM_NCHITTEST,
        WM_SIZE, WM_STYLECHANGING, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING, WS_EX_APPWINDOW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    use super::UnderlayError;

    const PARENT_SUBCLASS_ID: usize = 0x4E_41_55_50;
    const UNDERLAY_SUBCLASS_ID: usize = 0x4E_41_55_55;

    fn hwnd<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return None;
        };
        Some(handle.hwnd.get() as HWND)
    }

    /// Shared with both subclasses; freed only after both are removed.
    struct Placement {
        parent: HWND,
        underlay: HWND,
        cloaked: std::cell::Cell<bool>,
    }

    impl Placement {
        fn sync(&self) {
            // SAFETY: both windows are live while the subclasses hold this.
            let hidden = unsafe { IsIconic(self.parent) != 0 || IsWindowVisible(self.parent) == 0 };
            // Hidden by cloaking, not by `WS_VISIBLE`: winit owns that flag
            // for the underlay and would write it back on its next style
            // change. A cloaked window is also out of capture lists, as a
            // minimized one is.
            if hidden != self.cloaked.get() {
                let cloak: i32 = hidden.into();
                unsafe {
                    DwmSetWindowAttribute(
                        self.underlay,
                        DWMWA_CLOAK as u32,
                        std::ptr::from_ref(&cloak).cast(),
                        std::mem::size_of::<i32>() as u32,
                    );
                }
                self.cloaked.set(hidden);
            }
            if hidden {
                return;
            }
            let mut origin = POINT { x: 0, y: 0 };
            let mut client = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            let mut current = client;
            unsafe {
                ClientToScreen(self.parent, &mut origin);
                GetClientRect(self.parent, &mut client);
                GetWindowRect(self.underlay, &mut current);
            }
            let target = RECT {
                left: origin.x,
                top: origin.y,
                right: origin.x + client.right,
                bottom: origin.y + client.bottom,
            };
            let below = unsafe { GetWindow(self.parent, GW_HWNDNEXT) } == self.underlay;
            let placed = current.left == target.left
                && current.top == target.top
                && current.right == target.right
                && current.bottom == target.bottom;
            // Reordering a composition window that is already in place makes
            // DWM show it with no content for a frame; only move what moved.
            if below && placed {
                return;
            }
            // Inserted right after the parent: directly below it, in its band.
            unsafe {
                SetWindowPos(
                    self.underlay,
                    self.parent,
                    target.left,
                    target.top,
                    target.right - target.left,
                    target.bottom - target.top,
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
        }
    }

    unsafe extern "system" fn parent_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: forward first, so the parent has its new geometry.
        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        // SAFETY: `data` is the placement installed with this subclass; it is
        // removed before the placement is freed.
        let placement = unsafe { &*(data as *const Placement) };
        match message {
            WM_WINDOWPOSCHANGED | WM_SIZE => placement.sync(),
            WM_NCDESTROY => unsafe {
                RemoveWindowSubclass(hwnd, Some(parent_proc), PARENT_SUBCLASS_ID);
            },
            _ => {}
        }
        result
    }

    unsafe extern "system" fn underlay_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        // SAFETY: as in `parent_proc`.
        let placement = unsafe { &*(data as *const Placement) };
        match message {
            WM_NCHITTEST => return HTTRANSPARENT as LRESULT,
            WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
            // Whatever reorders the underlay puts it back under the parent.
            WM_WINDOWPOSCHANGING => {
                // SAFETY: lparam is the WINDOWPOS this message carries.
                let pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
                if pos.flags & SWP_NOZORDER == 0 {
                    pos.hwndInsertAfter = placement.parent;
                }
            }
            // winit rewrites the extended style from its own flags; keep the
            // underlay out of activation, the taskbar and Alt+Tab. Not a tool
            // window: capture software leaves those out of its window list.
            WM_STYLECHANGING if wparam as i32 == GWL_EXSTYLE => {
                // SAFETY: lparam is the STYLESTRUCT this message carries.
                let style = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
                style.styleNew =
                    (style.styleNew | WS_EX_NOACTIVATE) & !(WS_EX_APPWINDOW | WS_EX_TOOLWINDOW);
            }
            WM_NCDESTROY => unsafe {
                RemoveWindowSubclass(hwnd, Some(underlay_proc), UNDERLAY_SUBCLASS_ID);
            },
            _ => {}
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    pub(super) struct Underlay {
        placement: *mut Placement,
    }

    impl Underlay {
        pub(super) fn attach<P, U>(parent: &P, underlay: &U) -> Result<Self, UnderlayError>
        where
            P: HasWindowHandle + ?Sized,
            U: HasWindowHandle + ?Sized,
        {
            let failed = |reason: &str| UnderlayError::Failed(reason.into());
            let parent = hwnd(parent).ok_or_else(|| failed("parent has no HWND"))?;
            let underlay = hwnd(underlay).ok_or_else(|| failed("underlay has no HWND"))?;
            let placement = Box::into_raw(Box::new(Placement {
                parent,
                underlay,
                cloaked: std::cell::Cell::new(false),
            }));
            // SAFETY: `placement` outlives both subclasses; Drop removes them
            // before freeing it.
            let installed = unsafe {
                SetWindowSubclass(
                    underlay,
                    Some(underlay_proc),
                    UNDERLAY_SUBCLASS_ID,
                    placement as usize,
                ) != 0
                    && SetWindowSubclass(
                        parent,
                        Some(parent_proc),
                        PARENT_SUBCLASS_ID,
                        placement as usize,
                    ) != 0
            };
            if !installed {
                unsafe {
                    RemoveWindowSubclass(underlay, Some(underlay_proc), UNDERLAY_SUBCLASS_ID);
                    drop(Box::from_raw(placement));
                }
                return Err(failed("could not subclass the windows"));
            }
            // Re-apply the extended style through the subclass above.
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetWindowLongPtrW, SetWindowLongPtrW,
            };
            unsafe {
                let style = GetWindowLongPtrW(underlay, GWL_EXSTYLE);
                SetWindowLongPtrW(underlay, GWL_EXSTYLE, style);
            }
            let this = Self { placement };
            this.sync();
            Ok(this)
        }

        pub(super) fn sync(&self) {
            // SAFETY: the placement lives as long as `self`.
            unsafe { &*self.placement }.sync();
        }
    }

    impl Drop for Underlay {
        fn drop(&mut self) {
            // SAFETY: the placement lives until here; both subclasses go first.
            unsafe {
                let placement = &*self.placement;
                RemoveWindowSubclass(placement.parent, Some(parent_proc), PARENT_SUBCLASS_ID);
                RemoveWindowSubclass(
                    placement.underlay,
                    Some(underlay_proc),
                    UNDERLAY_SUBCLASS_ID,
                );
                drop(Box::from_raw(self.placement));
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use raw_window_handle::HasWindowHandle;

    use super::UnderlayError;

    pub(super) struct Underlay;

    impl Underlay {
        pub(super) fn attach<P, U>(_parent: &P, _underlay: &U) -> Result<Self, UnderlayError>
        where
            P: HasWindowHandle + ?Sized,
            U: HasWindowHandle + ?Sized,
        {
            Err(UnderlayError::Unsupported(
                "this backend cannot keep one window glued beneath another".into(),
            ))
        }

        pub(super) fn sync(&self) {}
    }
}
