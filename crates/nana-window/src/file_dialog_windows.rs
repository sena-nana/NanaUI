//! Common Item Dialog (`IFileOpenDialog` / `IFileSaveDialog`) on a worker
//! thread. HRESULT cancel is distinct from a platform failure.
use super::*;
use crate::win32::wide;
use raw_window_handle::RawWindowHandle;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FILEOPENDIALOGOPTIONS, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
    FOS_NOCHANGEDIR, FOS_NOREADONLYRETURN, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS,
    FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, PCWSTR};

#[path = "file_dialog_windows_types.rs"]
mod types;

#[path = "file_dialog_windows_events.rs"]
mod events;

const CANCELLED: HRESULT = HRESULT::from_win32(ERROR_CANCELLED.0);

struct ComScope;

impl ComScope {
    fn enter() -> Result<Self, FileDialogError> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if hr.is_err() {
            return Err(FileDialogError::Platform(format!(
                "CoInitializeEx failed: {hr:?}"
            )));
        }
        Ok(Self)
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

enum NativeDialog {
    Open(IFileOpenDialog),
    Save(IFileSaveDialog),
}

impl NativeDialog {
    fn create(kind: FileDialogKind) -> Result<Self, FileDialogError> {
        unsafe {
            if kind.is_save() {
                CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
                    .map(Self::Save)
                    .map_err(platform)
            } else {
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                    .map(Self::Open)
                    .map_err(platform)
            }
        }
    }

    fn file_dialog(&self) -> &IFileDialog {
        match self {
            Self::Open(dialog) => dialog,
            Self::Save(dialog) => dialog,
        }
    }

    fn collect_paths(&self, multiple: bool) -> Result<Vec<PathBuf>, FileDialogError> {
        unsafe {
            if multiple {
                let Self::Open(open) = self else {
                    return Err(FileDialogError::Platform(
                        "multi-select requires an open dialog".into(),
                    ));
                };
                let items = open.GetResults().map_err(platform)?;
                let count = items.GetCount().map_err(platform)?;
                let mut paths = Vec::with_capacity(count as usize);
                for index in 0..count {
                    let item = items.GetItemAt(index).map_err(platform)?;
                    paths.push(shell_item_path(&item)?);
                }
                Ok(paths)
            } else {
                let item = self.file_dialog().GetResult().map_err(platform)?;
                Ok(vec![shell_item_path(&item)?])
            }
        }
    }
}

pub(super) fn open<W>(
    window: Arc<W>,
    request: FileDialogRequest,
    completion: impl FnOnce(FileDialogResult) + Send + 'static,
) -> Result<FileDialogHandle, FileDialogError>
where
    W: HasWindowHandle + HasDisplayHandle + Send + Sync + ?Sized + 'static,
{
    // winit 0.31 `HasWindowHandle` is only valid on the window thread. Capture
    // the owner HWND here; the worker keeps `window` alive for Show().
    let Some(owner) = owner_hwnd(window.as_ref()) else {
        return Err(FileDialogError::WindowClosed);
    };
    // HWND is a raw pointer and not Send; the integer is the portable owner id.
    let owner = owner.0 as isize;
    let cancellation = crate::platform::DialogCancellation::default();
    let cancel = cancellation.clone();
    std::thread::Builder::new()
        .name("nana-file-dialog".into())
        .spawn(move || {
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
                present(HWND(owner as *mut _), &request)
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

pub(super) fn describe(
    request: &FileDialogRequest,
) -> Option<(Option<String>, Option<String>, Vec<String>)> {
    let configured = inspect(request).ok()?;
    Some((
        configured.title,
        configured.directory,
        configured.extensions,
    ))
}

fn present(owner: HWND, request: &FileDialogRequest) -> FileDialogResult {
    let _com = match ComScope::enter() {
        Ok(scope) => scope,
        Err(error) => return FileDialogResult::failed(request.id, error),
    };
    let dialog = match NativeDialog::create(request.kind) {
        Ok(dialog) => dialog,
        Err(error) => return FileDialogResult::failed(request.id, error),
    };
    if let Err(error) = configure(&dialog, request) {
        return FileDialogResult::failed(request.id, error);
    }
    let _types = match types::subscribe(dialog.file_dialog(), request) {
        Ok(subscription) => subscription,
        Err(error) => return FileDialogResult::failed(request.id, platform(error)),
    };
    match unsafe { dialog.file_dialog().Show(Some(owner)) } {
        Ok(()) => match dialog.collect_paths(request.kind.is_multiple()) {
            Ok(paths) => match require_directories(request.kind, paths) {
                Ok(paths) => FileDialogResult::selected(request.id, paths).with_selected_filter(
                    types::selected_filter(dialog.file_dialog(), request.filters.len()),
                ),
                Err(error) => FileDialogResult::failed(request.id, error),
            },
            Err(error) => FileDialogResult::failed(request.id, error),
        },
        Err(error) => result_from_hresult(request.id, error.code()),
    }
}

fn configure(dialog: &NativeDialog, request: &FileDialogRequest) -> Result<(), FileDialogError> {
    let file = dialog.file_dialog();
    let options = unsafe { file.GetOptions() }.map_err(platform)? | dialog_options(request.kind);
    unsafe {
        file.SetOptions(options).map_err(platform)?;
        if let Some(title) = &request.title {
            let title = wide(title);
            file.SetTitle(PCWSTR(title.as_ptr())).map_err(platform)?;
        }
    }
    apply_directory(file, request.directory.as_deref());
    if matches!(
        request.kind,
        FileDialogKind::PickFolder | FileDialogKind::PickFolders
    ) {
        return Ok(());
    }
    if let Some(name) = &request.file_name {
        let name = wide(name);
        unsafe {
            file.SetFileName(PCWSTR(name.as_ptr())).map_err(platform)?;
        }
    }
    apply_filters(file, &request.filters)
}

/// Folder kinds never apply the request's types, so they report none.
fn selected_type(dialog: &IFileDialog, request: &FileDialogRequest) -> Option<usize> {
    if matches!(
        request.kind,
        FileDialogKind::PickFolder | FileDialogKind::PickFolders
    ) {
        return None;
    }
    events::selected_filter(dialog, request.filters.len())
}

fn apply_directory(dialog: &IFileDialog, directory: Option<&Path>) {
    let Some(directory) = directory else {
        return;
    };
    // Non-directories (missing, file, or otherwise not a folder) must not
    // become SetFolder; SHCreateItemFromParsingName succeeds for those too.
    if !directory.is_dir() {
        return;
    }
    let wide = win32_parsing_name(directory);
    let item =
        unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None) };
    let Ok(item) = item else {
        return;
    };
    let _ = unsafe { dialog.SetFolder(&item) };
}

fn require_directories(
    kind: FileDialogKind,
    paths: Vec<PathBuf>,
) -> Result<Vec<PathBuf>, FileDialogError> {
    if !matches!(
        kind,
        FileDialogKind::PickFolder | FileDialogKind::PickFolders
    ) {
        return Ok(paths);
    }
    for path in &paths {
        if path.exists() && !path.is_dir() {
            return Err(FileDialogError::Platform(format!(
                "folder dialog returned a non-directory: {}",
                path.display()
            )));
        }
    }
    Ok(paths)
}

fn apply_filters(dialog: &IFileDialog, filters: &[FileFilter]) -> Result<(), FileDialogError> {
    if filters.is_empty() {
        return Ok(());
    }
    let mut names = Vec::new();
    let mut specs = Vec::new();
    for filter in filters {
        names.push(wide(&filter.name));
        let pattern = filter
            .extensions
            .iter()
            .map(|extension| format!("*.{extension}"))
            .collect::<Vec<_>>()
            .join(";");
        specs.push(wide(&pattern));
    }
    let list: Vec<COMDLG_FILTERSPEC> = names
        .iter()
        .zip(&specs)
        .map(|(name, spec)| COMDLG_FILTERSPEC {
            pszName: PCWSTR(name.as_ptr()),
            pszSpec: PCWSTR(spec.as_ptr()),
        })
        .collect();
    unsafe {
        dialog.SetFileTypes(&list).map_err(platform)?;
        // The first type starts selected; a type change follows in `events`. The index reads 0
        // until it is set or the dialog is shown, so set it here.
        dialog.SetFileTypeIndex(1).map_err(platform)?;
        if let Some(extension) = default_extension(&filters[0]) {
            let extension = wide(extension);
            dialog
                .SetDefaultExtension(PCWSTR(extension.as_ptr()))
                .map_err(platform)?;
        }
    }
    Ok(())
}

fn dialog_options(kind: FileDialogKind) -> FILEOPENDIALOGOPTIONS {
    let base = FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR | FOS_PATHMUSTEXIST;
    match kind {
        FileDialogKind::OpenFile => base | FOS_FILEMUSTEXIST,
        FileDialogKind::OpenFiles => base | FOS_FILEMUSTEXIST | FOS_ALLOWMULTISELECT,
        FileDialogKind::SaveFile => base | FOS_OVERWRITEPROMPT | FOS_NOREADONLYRETURN,
        FileDialogKind::PickFolder => base | FOS_PICKFOLDERS | FOS_FILEMUSTEXIST,
        FileDialogKind::PickFolders => {
            base | FOS_PICKFOLDERS | FOS_FILEMUSTEXIST | FOS_ALLOWMULTISELECT
        }
    }
}

fn result_from_hresult(id: u64, hr: HRESULT) -> FileDialogResult {
    if hr == CANCELLED {
        FileDialogResult::cancelled(id)
    } else {
        FileDialogResult::failed(
            id,
            FileDialogError::Platform(format!("HRESULT {:#010x}", hr.0 as u32)),
        )
    }
}

struct Configured {
    title: Option<String>,
    directory: Option<String>,
    extensions: Vec<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    options: FILEOPENDIALOGOPTIONS,
    #[cfg_attr(not(test), allow(dead_code))]
    file_name: Option<String>,
}

fn inspect(request: &FileDialogRequest) -> Result<Configured, FileDialogError> {
    let _com = ComScope::enter()?;
    let dialog = NativeDialog::create(request.kind)?;
    configure(&dialog, request)?;
    let file = dialog.file_dialog();
    let options = unsafe { file.GetOptions() }.map_err(platform)?;
    let directory = unsafe { file.GetFolder().ok() }.and_then(|item| {
        shell_item_path(&item)
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
    });
    Ok(Configured {
        title: request.title.as_ref().map(|title| title.to_string()),
        directory,
        extensions: match request.kind {
            FileDialogKind::PickFolder | FileDialogKind::PickFolders => Vec::new(),
            _ => request
                .filters
                .iter()
                .flat_map(|filter| {
                    filter
                        .extensions
                        .iter()
                        .map(|extension| extension.to_string())
                })
                .collect(),
        },
        options,
        file_name: dialog_file_name(file),
    })
}

fn dialog_file_name(dialog: &IFileDialog) -> Option<String> {
    let name = unsafe { dialog.GetFileName().ok()? };
    if name.is_null() {
        return None;
    }
    let text = unsafe {
        let wide = name.as_wide();
        let text = String::from_utf16_lossy(wide);
        CoTaskMemFree(Some(name.0 as *const _));
        text
    };
    let text = text.trim_end_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn shell_item_path(item: &IShellItem) -> Result<PathBuf, FileDialogError> {
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(platform)?;
    if name.is_null() {
        return Err(FileDialogError::Platform(
            "shell item has no filesystem path".into(),
        ));
    }
    let path = unsafe {
        let wide = name.as_wide();
        let path = PathBuf::from(std::ffi::OsString::from_wide(wide));
        CoTaskMemFree(Some(name.0 as *const _));
        path
    };
    Ok(path)
}

fn owner_hwnd<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// Prefer Shell-compatible spelling without changing which directory is meant.
/// A verbatim-only path may be rejected by the Shell; configure then skips it.
fn win32_parsing_name(directory: &Path) -> Vec<u16> {
    let path = nana_ui_platform::path::normalize_path(directory);
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    name.push(0);
    name
}

fn platform(error: impl std::fmt::Display) -> FileDialogError {
    FileDialogError::Platform(error.to_string())
}

#[cfg(test)]
#[path = "file_dialog_windows_tests.rs"]
mod tests;
