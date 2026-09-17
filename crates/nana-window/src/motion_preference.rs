//! The system "reduce motion" accessibility preference.
//!
//! Windows reports it as "Show animations in Windows"
//! (`SPI_GETCLIENTAREAANIMATION`) and broadcasts `WM_SETTINGCHANGE` with
//! `SPI_SETCLIENTAREAANIMATION` when the user flips it; the host's per-window
//! subclass records that broadcast. Other platforms do not report it yet.

use std::sync::atomic::{AtomicBool, Ordering};

static CHANGED: AtomicBool = AtomicBool::new(false);

#[cfg(any(target_os = "windows", test))]
const SPI_SETCLIENTAREAANIMATION: usize = 0x1043;

/// Whether the user asked the system to reduce motion; `None` when the
/// platform does not report the preference.
pub fn system_reduced_motion() -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
        };
        let mut animations = 0i32;
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to pvParam.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&raw mut animations).cast(),
                0,
            )
        };
        (ok != 0).then_some(animations == 0)
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Whether the preference may have changed since the last call.
pub fn take_reduced_motion_change() -> bool {
    CHANGED.swap(false, Ordering::AcqRel)
}

/// Record a `WM_SETTINGCHANGE` broadcast whose `wparam` names the setting.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn observe_setting_change(wparam: usize) {
    if wparam == SPI_SETCLIENTAREAANIMATION {
        CHANGED.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_animation_setting_marks_a_change_and_taking_clears_it() {
        let _ = take_reduced_motion_change();
        observe_setting_change(0x0030);
        assert!(!take_reduced_motion_change());
        observe_setting_change(SPI_SETCLIENTAREAANIMATION);
        assert!(take_reduced_motion_change());
        assert!(!take_reduced_motion_change());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_reports_the_preference() {
        assert!(system_reduced_motion().is_some());
    }
}
