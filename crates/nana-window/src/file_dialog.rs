//! System file dialog, opened by the host on the application's behalf.
//!
//! The request and result model is in `nana-ui-core`; this is the platform
//! half. A dialog needs the parent window handle so it can hang off the right
//! window — on macOS as a sheet, on Windows as an owned modal — and that
//! handle only exists in the host, which is why this is not a control API.
//! `PathField` still just emits `BrowseRequested`.
//!
//! Results arrive asynchronously: the dialog must not block the event loop, or
//! the window behind it stops rendering. Drain [`take_file_dialog_results`]
//! once a frame, the same way menu activations are drained.

use std::sync::{Mutex, OnceLock};

pub use nana_ui_core::{FileDialogKind, FileDialogRequest, FileDialogResult, FileFilter};

/// How much of the file dialog the running platform provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogSupport {
    /// The system dialog opens.
    System,
    /// Nothing opens; every request is answered with a cancel so a caller
    /// waiting on a result is not left hanging.
    Unavailable,
}

/// What [`open_file_dialog`] will do on this platform.
pub const fn file_dialog_support() -> FileDialogSupport {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        FileDialogSupport::System
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        FileDialogSupport::Unavailable
    }
}

fn results() -> &'static Mutex<Vec<FileDialogResult>> {
    static RESULTS: OnceLock<Mutex<Vec<FileDialogResult>>> = OnceLock::new();
    RESULTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Records an outcome. Called from the platform's completion handler.
pub(crate) fn push_result(result: FileDialogResult) {
    if let Ok(mut queue) = results().lock() {
        queue.push(result);
    }
}

/// File dialog outcomes since the last call, in completion order.
///
/// A cancelled dialog still produces a result, so a caller can drop whatever
/// it was holding for that request instead of waiting forever.
pub fn take_file_dialog_results() -> Vec<FileDialogResult> {
    results()
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

/// Opens the system file dialog for `request` on `window`.
///
/// Returns immediately; the outcome shows up in [`take_file_dialog_results`].
/// On a platform without a dialog the request is answered with a cancel right
/// away rather than silently dropped.
pub fn open_file_dialog<W: raw_window_handle::HasWindowHandle + ?Sized>(
    window: &W,
    request: FileDialogRequest,
) -> FileDialogSupport {
    let _ = window;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        crate::platform::open_file_dialog(window, request);
        FileDialogSupport::System
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        push_result(FileDialogResult::cancelled(request.id));
        FileDialogSupport::Unavailable
    }
}

/// Reads back how the platform configured a dialog for `request`, without
/// presenting it: title, starting directory, allowed extensions.
///
/// A modal dialog cannot be driven from a test, so this verifies the half that
/// is ours — that the request reached the platform intact. Returns `None`
/// where the platform cannot be queried.
pub fn describe_configured_dialog(
    request: &FileDialogRequest,
) -> Option<(Option<String>, Option<String>, Vec<String>)> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::describe_configured_panel(request)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_drain_in_order_and_leave_the_queue_empty() {
        let _ = take_file_dialog_results();
        push_result(FileDialogResult {
            id: 1,
            paths: vec!["/tmp/a".into()],
        });
        push_result(FileDialogResult::cancelled(2));
        let drained = take_file_dialog_results();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].id, 1);
        assert!(drained[1].is_cancelled());
        assert!(take_file_dialog_results().is_empty());
    }
}
