//! Window-owned system dialogs. Completion invokes the host's callback once;
//! no process-global queue or per-frame polling is involved.

pub use nana_ui_core::{
    FileDialogError, FileDialogKind, FileDialogRequest, FileDialogResult, FileFilter,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogSupport {
    System,
    Unavailable,
}

pub const fn file_dialog_support() -> FileDialogSupport {
    if cfg!(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    )) {
        FileDialogSupport::System
    } else {
        FileDialogSupport::Unavailable
    }
}

/// Owns the native presentation. Dropping it dismisses an unfinished picker.
/// Keep it on the host thread (AppKit sheet teardown is main-thread only).
pub struct FileDialogHandle(Option<Box<dyn FnOnce()>>);
impl FileDialogHandle {
    pub(crate) fn new(cancel: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(cancel)))
    }
}
impl Drop for FileDialogHandle {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            cancel();
        }
    }
}

/// Open a dialog without blocking the host event loop. The caller retains
/// request/window identity and must reject callbacks for closed windows.
/// A retained parent keeps raw handles valid for the worker's lifetime.
pub fn open_file_dialog<W>(
    window: Arc<W>,
    request: FileDialogRequest,
    completion: impl FnOnce(FileDialogResult) + Send + 'static,
) -> Result<FileDialogHandle, FileDialogError>
where
    W: HasWindowHandle + HasDisplayHandle + Send + Sync + ?Sized + 'static,
{
    window
        .window_handle()
        .map_err(|_| FileDialogError::WindowClosed)?;
    #[cfg(target_os = "macos")]
    {
        crate::platform::open_file_dialog(window.as_ref(), request, Box::new(completion))
    }
    #[cfg(target_os = "windows")]
    {
        windows_dialog::open(window, request, completion)
    }
    #[cfg(target_os = "linux")]
    {
        linux::open(window, request, completion)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (window, request, completion);
        Err(FileDialogError::Unavailable)
    }
}

/// Reads back how the platform configured a dialog for `request`, without
/// presenting it: title, starting directory, allowed extensions.
///
/// This complements hosted interaction checks by inspecting configuration
/// without displaying a native picker. Returns `None` where the platform
/// cannot be queried.
///
/// On Windows, directory comes from `GetFolder`. Title is echoed after
/// `SetTitle` (no COM getter). Folder kinds report no extensions.
pub fn describe_configured_dialog(
    request: &FileDialogRequest,
) -> Option<(Option<String>, Option<String>, Vec<String>)> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::describe_configured_panel(request)
    }
    #[cfg(target_os = "windows")]
    {
        windows_dialog::describe(request)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = request;
        None
    }
}

#[cfg(target_os = "linux")]
#[path = "file_dialog_linux.rs"]
mod linux;

#[cfg(target_os = "windows")]
#[path = "file_dialog_windows.rs"]
mod windows_dialog;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn releasing_a_native_session_runs_its_cancellation_once() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = count.clone();
        let handle = FileDialogHandle::new(move || {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);
        drop(handle);
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
