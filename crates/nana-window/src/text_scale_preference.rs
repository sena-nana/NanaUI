//! The system text-size preference.
//!
//! Windows reports "Text size" (Settings > Accessibility) as
//! `UISettings.TextScaleFactor`, 1.0 to 2.25, and raises
//! `UISettings.TextScaleFactorChanged` on a worker thread when the user moves
//! it. macOS has no system-wide text size (Dynamic Type is iOS only), Linux
//! desktops have no portable one: both report `None`, which callers treat as
//! 1.0. NanaUI does not apply the factor itself; an application that offers
//! "follow system" scaling reads it and sets its own zoom.

use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "windows")]
use std::sync::{Mutex, OnceLock};

static CHANGED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "windows")]
type Waker = Box<dyn Fn() + Send + Sync>;

/// Called from the change handler's thread; `None` until a host installs one.
#[cfg(target_os = "windows")]
static WAKER: Mutex<Option<Waker>> = Mutex::new(None);

/// The system text-size factor, `1.0` meaning the default size. `None` when
/// the platform has no system text size (macOS, Linux) or the read failed.
pub fn system_text_scale() -> Option<f32> {
    #[cfg(target_os = "windows")]
    {
        let settings = ui_settings()?;
        settings
            .TextScaleFactor()
            .ok()
            .and_then(|factor| normalized_text_scale(factor as f32))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Whether the factor may have changed since the last call.
pub fn take_text_scale_change() -> bool {
    CHANGED.swap(false, Ordering::AcqRel)
}

/// Install what wakes the host's event loop when the factor changes. The
/// Windows change event arrives on a worker thread, outside the window's
/// message stream, so without a waker the change waits for the next event.
/// Other platforms never call it. A later call replaces the earlier waker.
pub fn set_text_scale_waker(waker: impl Fn() + Send + Sync + 'static) {
    #[cfg(target_os = "windows")]
    {
        *WAKER.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(Box::new(waker));
        // Subscribe now, so a change before the first read is not missed.
        let _ = ui_settings();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = waker;
    }
}

/// A factor the host can use: finite and positive.
#[cfg(any(target_os = "windows", test))]
fn normalized_text_scale(factor: f32) -> Option<f32> {
    (factor.is_finite() && factor > 0.0).then_some(factor)
}

#[cfg(any(target_os = "windows", test))]
fn mark_changed() {
    CHANGED.store(true, Ordering::Release);
}

/// One `UISettings` for the process, subscribed to its change event. The
/// subscription lives as long as the object, which is the process.
#[cfg(target_os = "windows")]
fn ui_settings() -> Option<&'static windows::UI::ViewManagement::UISettings> {
    use windows::Foundation::TypedEventHandler;
    use windows::UI::ViewManagement::UISettings;

    static SETTINGS: OnceLock<Option<UISettings>> = OnceLock::new();
    SETTINGS
        .get_or_init(|| {
            let settings = UISettings::new().ok()?;
            let _ = settings.TextScaleFactorChanged(&TypedEventHandler::new(|_, _| {
                mark_changed();
                if let Some(wake) = WAKER
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .as_ref()
                {
                    wake();
                }
                Ok(())
            }));
            Some(settings)
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_change_is_taken_once() {
        let _serial = crate::test_serial_lock();
        let _ = take_text_scale_change();
        assert!(!take_text_scale_change());
        mark_changed();
        assert!(take_text_scale_change());
        assert!(!take_text_scale_change());
    }

    #[test]
    fn only_a_finite_positive_factor_is_reported() {
        assert_eq!(normalized_text_scale(1.25), Some(1.25));
        assert_eq!(normalized_text_scale(0.0), None);
        assert_eq!(normalized_text_scale(f32::NAN), None);
    }

    #[test]
    fn platforms_without_a_system_text_size_report_none() {
        if !cfg!(target_os = "windows") {
            assert_eq!(system_text_scale(), None);
        }
    }
}
