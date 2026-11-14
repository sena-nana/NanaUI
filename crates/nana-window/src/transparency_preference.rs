//! The system "reduce transparency" accessibility preference.
//!
//! macOS reports "Reduce transparency"
//! (`accessibilityDisplayShouldReduceTransparency`) and its changes through
//! the workspace display-options notification (see `display_options`). Other
//! platforms do not report it: Windows' "Transparency effects" switch is a
//! personalization setting, not an accessibility preference NanaUI reads.

use std::sync::atomic::{AtomicBool, Ordering};

static CHANGED: AtomicBool = AtomicBool::new(false);

/// Whether the user asked the system to reduce transparency; `None` when the
/// platform does not report the preference.
pub fn system_reduced_transparency() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        Some(crate::display_options::display_option(
            crate::display_options::DisplayOption::ReduceTransparency,
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Mark the preference changed; the platform observer calls it.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn mark_changed() {
    CHANGED.store(true, Ordering::Release);
}

/// Whether the preference may have changed since the last call.
pub fn take_reduced_transparency_change() -> bool {
    CHANGED.swap(false, Ordering::AcqRel)
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test owns CHANGED: parallel tests taking it would race.
    #[test]
    fn a_marked_change_is_taken_once_and_the_macos_notification_marks_it() {
        let _serial = crate::test_serial_lock();
        let _ = take_reduced_transparency_change();
        assert!(!take_reduced_transparency_change());
        mark_changed();
        assert!(take_reduced_transparency_change());
        assert!(!take_reduced_transparency_change());
        #[cfg(target_os = "macos")]
        {
            // The first read installs the observer; a post runs it at once.
            let _ = system_reduced_transparency();
            crate::display_options::post_change_for_test();
            assert!(take_reduced_transparency_change());
        }
    }

    #[test]
    fn only_macos_reports_the_preference() {
        assert_eq!(
            system_reduced_transparency().is_some(),
            cfg!(target_os = "macos")
        );
    }
}
