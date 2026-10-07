//! The system high-contrast accessibility flag.
//!
//! Windows reports it with `SPI_GETHIGHCONTRAST` (`HCF_HIGHCONTRASTON`) and
//! broadcasts `WM_SETTINGCHANGE` with `SPI_SETHIGHCONTRAST` when the user
//! flips it. The host's per-window subclass records that broadcast. Other
//! platforms do not report it yet.
//!
//! This is not DirectWrite text gamma (`system_contrast`).

use std::sync::atomic::{AtomicU8, Ordering};

/// `0` follows the platform. `1` forces off. `2` forces on.
static FORCED: AtomicU8 = AtomicU8::new(0);

#[cfg(any(target_os = "windows", test))]
const SPI_SETHIGHCONTRAST: usize = 0x0043;

static CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the operating system is painting in a high-contrast theme.
///
/// `None` when the platform does not report the flag. A test may force the
/// value with [`force_high_contrast`]; `None` there returns to the platform.
pub fn system_high_contrast() -> Option<bool> {
    match FORCED.load(Ordering::Acquire) {
        1 => return Some(false),
        2 => return Some(true),
        _ => {}
    }
    platform_high_contrast()
}

/// Force the flag tests and hosts observe. `None` reads the platform again.
pub fn force_high_contrast(value: Option<bool>) {
    FORCED.store(
        match value {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        },
        Ordering::Release,
    );
}

fn platform_high_contrast() -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::UI::WindowsAndMessaging::SystemParametersInfoW;

        const SPI_GETHIGHCONTRAST: u32 = 0x0042;
        const HCF_HIGHCONTRASTON: u32 = 0x0000_0001;

        #[repr(C)]
        struct HighContrastW {
            cb_size: u32,
            flags: u32,
            scheme: *mut u16,
        }

        let mut info = HighContrastW {
            cb_size: std::mem::size_of::<HighContrastW>() as u32,
            flags: 0,
            scheme: std::ptr::null_mut(),
        };
        // SAFETY: SPI_GETHIGHCONTRAST writes one HIGHCONTRASTW. The scheme
        // string, when present, is a LocalAlloc the caller frees.
        let ok = unsafe {
            SystemParametersInfoW(SPI_GETHIGHCONTRAST, info.cb_size, (&raw mut info).cast(), 0)
        };
        if !info.scheme.is_null() {
            // SAFETY: the system allocated this scheme string for the caller.
            unsafe { LocalFree(info.scheme.cast()) };
        }
        (ok != 0).then_some(info.flags & HCF_HIGHCONTRASTON != 0)
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Whether the flag may have changed since the last call.
pub fn take_high_contrast_change() -> bool {
    CHANGED.swap(false, Ordering::AcqRel)
}

/// Record a `WM_SETTINGCHANGE` broadcast whose `wparam` names the setting.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn observe_setting_change(wparam: usize) {
    if wparam == SPI_SETHIGHCONTRAST {
        CHANGED.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_high_contrast_setting_marks_a_change_and_taking_clears_it() {
        let _ = take_high_contrast_change();
        observe_setting_change(0x0030);
        assert!(!take_high_contrast_change());
        observe_setting_change(SPI_SETHIGHCONTRAST);
        assert!(take_high_contrast_change());
        assert!(!take_high_contrast_change());
    }

    #[test]
    fn a_forced_flag_replaces_the_platform_read() {
        force_high_contrast(Some(true));
        assert_eq!(system_high_contrast(), Some(true));
        force_high_contrast(Some(false));
        assert_eq!(system_high_contrast(), Some(false));
        force_high_contrast(None);
        #[cfg(target_os = "windows")]
        assert!(system_high_contrast().is_some());
        #[cfg(not(target_os = "windows"))]
        assert!(system_high_contrast().is_none());
    }
}
