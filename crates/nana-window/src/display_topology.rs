//! Native display topology change notifications across desktop platforms.
//!
//! - macOS observes `NSApplicationDidChangeScreenParametersNotification` and
//!   sets the change flag + wakes the host event loop.
//! - Windows records `WM_DISPLAYCHANGE` and `WM_SETTINGCHANGE` (`SPI_SETWORKAREA`)
//!   inside [`crate::win32_hooks`] and sets the change flag.
//! - Linux relies on winit / Wayland output announcements checked during the
//!   host event loop's `about_to_wait` pass.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static CHANGED: AtomicBool = AtomicBool::new(false);
type Waker = Box<dyn Fn() + Send + Sync>;
static WAKER: Mutex<Option<Waker>> = Mutex::new(None);

/// Registers the host event-loop waker invoked when the platform notifies of
/// display hot-plug or geometry parameter changes.
pub fn set_display_topology_waker(waker: impl Fn() + Send + Sync + 'static) {
    if let Ok(mut slot) = WAKER.lock() {
        *slot = Some(Box::new(waker));
    }
    ensure_observer();
}

/// Returns whether a native display topology notification arrived since the
/// last call and clears the flag.
pub fn take_display_topology_change() -> bool {
    ensure_observer();
    CHANGED.swap(false, Ordering::AcqRel)
}

#[cfg_attr(
    not(any(target_os = "macos", target_os = "windows", test)),
    allow(dead_code)
)]
pub(crate) fn mark_changed() {
    CHANGED.store(true, Ordering::Release);
    if let Ok(slot) = WAKER.lock()
        && let Some(wake) = slot.as_ref()
    {
        wake();
    }
}

fn ensure_observer() {
    #[cfg(target_os = "macos")]
    macos::ensure_observer();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::Once;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2_foundation::NSString;

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSApplicationDidChangeScreenParametersNotification: &'static NSString;
    }

    pub(super) fn ensure_observer() {
        static INSTALLED: Once = Once::new();
        INSTALLED.call_once(|| {
            let block = block2::RcBlock::new(|_notification: std::ptr::NonNull<AnyObject>| {
                super::mark_changed();
            });
            // SAFETY: `[NSNotificationCenter defaultCenter]` lives for the
            // process; `null` queue dispatches synchronously on the posting
            // main thread.
            unsafe {
                let center: Retained<NSObject> =
                    objc2::msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
                let token: Option<Retained<NSObject>> = objc2::msg_send![
                    &*center,
                    addObserverForName: NSApplicationDidChangeScreenParametersNotification,
                    object: std::ptr::null::<AnyObject>(),
                    queue: std::ptr::null::<AnyObject>(),
                    usingBlock: &*block,
                ];
                std::mem::forget(token);
            }
        });
    }

    #[cfg(test)]
    pub(super) fn post_change_for_test() {
        unsafe {
            let center: Retained<NSObject> =
                objc2::msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
            let _: () = objc2::msg_send![
                &*center,
                postNotificationName: NSApplicationDidChangeScreenParametersNotification,
                object: std::ptr::null::<AnyObject>(),
            ];
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn marking_display_topology_sets_flag_and_fires_waker() {
        let _serial = crate::test_serial_lock();
        let _ = take_display_topology_change();
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_copy = Arc::clone(&wakes);
        set_display_topology_waker(move || {
            wakes_copy.fetch_add(1, Ordering::SeqCst);
        });
        mark_changed();
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert!(take_display_topology_change());
        assert!(!take_display_topology_change());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_screen_parameters_notification_triggers_display_topology_flag() {
        let _serial = crate::test_serial_lock();
        let _ = take_display_topology_change();
        macos::post_change_for_test();
        assert!(take_display_topology_change());
    }
}
