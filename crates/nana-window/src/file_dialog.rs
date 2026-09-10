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
        let mut dialog = rfd::FileDialog::new().set_parent(&window.as_ref());
        if let Some(title) = &request.title {
            dialog = dialog.set_title(title.as_ref());
        }
        if let Some(directory) = &request.directory {
            dialog = dialog.set_directory(directory);
        }
        if let Some(name) = &request.file_name {
            dialog = dialog.set_file_name(name.as_ref());
        }
        for filter in &request.filters {
            dialog = dialog.add_filter(filter.name.as_ref(), &filter.extensions);
        }
        let cancellation = crate::platform::DialogCancellation::default();
        let cancel = cancellation.clone();
        std::thread::Builder::new()
            .name("nana-file-dialog".into())
            .spawn(move || {
                // Keep the owner alive until the native dialog has stopped using it.
                let _parent = window;
                let _hook = match cancellation.install() {
                    Ok(hook) => hook,
                    Err(error) => {
                        completion(FileDialogResult::failed(request.id, error));
                        return;
                    }
                };
                if cancellation.cancelled() {
                    completion(FileDialogResult::cancelled(request.id));
                    return;
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let paths = match request.kind {
                        FileDialogKind::OpenFile => dialog.pick_file().map(|path| vec![path]),
                        FileDialogKind::OpenFiles => dialog.pick_files(),
                        FileDialogKind::SaveFile => dialog.save_file().map(|path| vec![path]),
                        FileDialogKind::PickFolder => dialog.pick_folder().map(|path| vec![path]),
                        FileDialogKind::PickFolders => dialog.pick_folders(),
                    };
                    // rfd does not expose an error channel; None is cancellation.
                    // Do not invent a platform error from that ambiguous outcome.
                    FileDialogResult::selected(request.id, paths.unwrap_or_default())
                }))
                .unwrap_or_else(|_| {
                    FileDialogResult::failed(
                        request.id,
                        FileDialogError::Platform("file dialog backend panicked".into()),
                    )
                });
                completion(result);
            })
            .map(|_| FileDialogHandle::new(move || cancel.cancel()))
            .map_err(|error| FileDialogError::Platform(error.to_string()))
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

#[cfg(target_os = "linux")]
#[path = "file_dialog_linux.rs"]
mod linux;

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
