//! Underlay windows: a window glued directly beneath another one.
//!
//! An underlay covers exactly its parent's client area, sits directly below
//! it in z-order (in the parent's band, so a topmost parent keeps a topmost
//! underlay), moves and resizes with it, hides while the parent is hidden or
//! minimized, and never takes input or activation: every click lands on the
//! parent above it. It has its own native surface and title so capture can
//! target its picture independently of the parent's controls. On macOS it
//! stays independent of the parent's attachment group so capture cannot
//! include the UI, even when the capturer includes attached windows.
//!
//! The parent's frame is the only authority; the underlay never persists or
//! constrains its own geometry. The host calls [`Underlay::sync`] on every
//! geometry, mode, focus or visibility change of the parent — not per frame:
//! on macOS synchronous AppKit notifications already follow native moves
//! and resizes; host sync also conforms visibility and ordering. On
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
    use std::cell::Cell;

    use objc2::rc::Retained;
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSApplicationDidBecomeActiveNotification, NSView, NSWindow, NSWindowCollectionBehavior,
        NSWindowDidBecomeKeyNotification, NSWindowDidBecomeMainNotification,
        NSWindowDidDeminiaturizeNotification, NSWindowDidMiniaturizeNotification,
        NSWindowDidMoveNotification, NSWindowDidResizeNotification, NSWindowOrderingMode,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject};
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

    struct Binding {
        parent: Retained<NSWindow>,
        window: Retained<NSWindow>,
        syncing: Cell<bool>,
    }

    impl Binding {
        fn sync(&self) {
            // Ordering can synchronously send more AppKit notifications.
            if self.syncing.replace(true) {
                return;
            }
            self.conform();
            self.syncing.set(false);
        }

        fn conform(&self) {
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
            // Independent windows: ScreenCaptureKit's includeChildWindows
            // otherwise captures the whole attachment group, including UI.
            window.orderWindow_relativeTo(NSWindowOrderingMode::Below, parent.windowNumber());
        }
    }

    define_class!(
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = Binding]
        struct UnderlayObserver;

        impl UnderlayObserver {
            #[unsafe(method(parentChanged:))]
            fn parent_changed(&self, _notification: &NSNotification) {
                // Runs inside AppKit's move/resize notification, before the
                // application event queue or GPU work can delay following.
                self.ivars().sync();
            }
        }
    );

    pub(super) struct Underlay {
        observer: Retained<UnderlayObserver>,
    }

    impl Underlay {
        pub(super) fn attach<P, U>(parent: &P, underlay: &U) -> Result<Self, UnderlayError>
        where
            P: HasWindowHandle + ?Sized,
            U: HasWindowHandle + ?Sized,
        {
            let failed = |reason: &str| UnderlayError::Failed(reason.into());
            let mtm = MainThreadMarker::new().ok_or_else(|| failed("not on main thread"))?;
            let parent = ns_window(parent).ok_or_else(|| failed("parent has no NSWindow"))?;
            let window = ns_window(underlay).ok_or_else(|| failed("underlay has no NSWindow"))?;
            parent.setIgnoresMouseEvents(false);
            window.setIgnoresMouseEvents(true);
            window.setHasShadow(false);
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::IgnoresCycle
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            let allocated = UnderlayObserver::alloc(mtm).set_ivars(Binding {
                parent,
                window,
                syncing: Cell::new(false),
            });
            // SAFETY: initialize our main-thread NSObject with installed ivars.
            let observer: Retained<UnderlayObserver> = unsafe { msg_send![super(allocated), init] };
            let center = NSNotificationCenter::defaultCenter();
            // SAFETY: AppKit posts these notifications on the main thread;
            // the retained observer implements parentChanged: and is removed
            // before its windows are released.
            unsafe {
                for name in [
                    NSWindowDidMoveNotification,
                    NSWindowDidResizeNotification,
                    NSWindowDidBecomeKeyNotification,
                    NSWindowDidBecomeMainNotification,
                    NSWindowDidMiniaturizeNotification,
                    NSWindowDidDeminiaturizeNotification,
                ] {
                    center.addObserver_selector_name_object(
                        &observer,
                        sel!(parentChanged:),
                        Some(name),
                        Some(&observer.ivars().parent),
                    );
                }
                center.addObserver_selector_name_object(
                    &observer,
                    sel!(parentChanged:),
                    Some(NSApplicationDidBecomeActiveNotification),
                    None,
                );
            }
            let this = Self { observer };
            this.sync();
            Ok(this)
        }

        pub(super) fn sync(&self) {
            self.observer.ivars().sync();
        }
    }

    impl Drop for Underlay {
        fn drop(&mut self) {
            // SAFETY: this observer was registered above and is still retained.
            unsafe { NSNotificationCenter::defaultCenter().removeObserver(&self.observer) };
            self.observer.ivars().window.orderOut(None);
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::cell::Cell;
    use std::rc::{Rc, Weak};

    use raw_window_handle::HasWindowHandle;
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GW_HWNDNEXT, GWL_EXSTYLE, GetClientRect, GetWindow, GetWindowLongPtrW, GetWindowRect,
        IsIconic, IsWindowVisible, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SetWindowLongPtrW,
        SetWindowPos,
    };

    use super::UnderlayError;
    use crate::win32_hooks::{UnderlayHookTarget, Win32WindowHooks};

    struct Placement {
        parent: HWND,
        underlay: HWND,
        cloaked: Cell<bool>,
    }

    impl Placement {
        fn sync_placement(&self) {
            // SAFETY: both windows are live while `Underlay` retains this.
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

    impl UnderlayHookTarget for Placement {
        fn parent_hwnd(&self) -> HWND {
            self.parent
        }

        fn sync(&self) {
            self.sync_placement();
        }
    }

    pub(super) struct Underlay {
        placement: Rc<Placement>,
        parent_hooks: Weak<Win32WindowHooks>,
        underlay_hooks: Weak<Win32WindowHooks>,
    }

    impl Underlay {
        pub(super) fn attach<P, U>(parent: &P, underlay: &U) -> Result<Self, UnderlayError>
        where
            P: HasWindowHandle + ?Sized,
            U: HasWindowHandle + ?Sized,
        {
            let failed = |reason: &str| UnderlayError::Failed(reason.into());
            let parent = crate::win32::hwnd(parent).ok_or_else(|| failed("parent has no HWND"))?;
            let underlay =
                crate::win32::hwnd(underlay).ok_or_else(|| failed("underlay has no HWND"))?;
            let underlay_hooks =
                Win32WindowHooks::install_or_get(underlay).map_err(UnderlayError::Failed)?;
            let parent_hooks =
                Win32WindowHooks::install_or_get(parent).map_err(UnderlayError::Failed)?;
            let placement = Rc::new(Placement {
                parent,
                underlay,
                cloaked: Cell::new(false),
            });
            let weak = Rc::downgrade(&placement) as Weak<dyn UnderlayHookTarget>;
            *underlay_hooks.underlay_self.borrow_mut() = Some(weak.clone());
            parent_hooks.underlay_children.borrow_mut().push(weak);
            // Re-apply the extended style through the unified hook above.
            unsafe {
                let style = GetWindowLongPtrW(underlay, GWL_EXSTYLE);
                SetWindowLongPtrW(underlay, GWL_EXSTYLE, style);
            }
            let this = Self {
                placement,
                parent_hooks: Rc::downgrade(&parent_hooks),
                underlay_hooks: Rc::downgrade(&underlay_hooks),
            };
            this.sync();
            Ok(this)
        }

        pub(super) fn sync(&self) {
            self.placement.sync_placement();
        }
    }

    impl Drop for Underlay {
        fn drop(&mut self) {
            let target = Rc::downgrade(&self.placement) as Weak<dyn UnderlayHookTarget>;
            if let Some(hooks) = self.parent_hooks.upgrade() {
                hooks
                    .underlay_children
                    .borrow_mut()
                    .retain(|weak| weak.strong_count() > 0 && !Weak::ptr_eq(weak, &target));
            }
            if let Some(hooks) = self.underlay_hooks.upgrade() {
                let mut slot = hooks.underlay_self.borrow_mut();
                if slot
                    .as_ref()
                    .is_some_and(|weak| Weak::ptr_eq(weak, &target))
                {
                    *slot = None;
                }
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
