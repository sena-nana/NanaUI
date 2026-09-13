//! Common Item Dialog (`IFileOpenDialog` / `IFileSaveDialog`) on a worker
//! thread. HRESULT cancel is distinct from a platform failure.
use super::*;
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
                present(_parent.as_ref(), &request)
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

fn present<W>(window: &W, request: &FileDialogRequest) -> FileDialogResult
where
    W: HasWindowHandle + ?Sized,
{
    let Some(owner) = owner_hwnd(window) else {
        return FileDialogResult::failed(request.id, FileDialogError::WindowClosed);
    };
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
    match unsafe { dialog.file_dialog().Show(Some(owner)) } {
        Ok(()) => match dialog.collect_paths(request.kind.is_multiple()) {
            Ok(paths) => FileDialogResult::selected(request.id, paths),
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
        if let Some(name) = &request.file_name {
            let name = wide(name);
            file.SetFileName(PCWSTR(name.as_ptr())).map_err(platform)?;
        }
    }
    apply_directory(file, request.directory.as_deref());
    if matches!(
        request.kind,
        FileDialogKind::PickFolder | FileDialogKind::PickFolders
    ) {
        return Ok(());
    }
    apply_filters(file, &request.filters)
}

fn apply_directory(dialog: &IFileDialog, directory: Option<&Path>) {
    let Some(directory) = directory else {
        return;
    };
    let wide = win32_parsing_name(directory);
    let item =
        unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None) };
    let Ok(item) = item else {
        return;
    };
    let _ = unsafe { dialog.SetFolder(&item) };
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
        if let Some(extension) = filters.iter().find_map(|filter| filter.extensions.first()) {
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
    if hr == ERROR_CANCELLED.to_hresult() {
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
    })
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

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `SHCreateItemFromParsingName` rejects `\\?\`. Map `\\?\C:\...` → `C:\...`
/// and `\\?\UNC\server\share` → `\\server\share`.
fn win32_parsing_name(directory: &Path) -> Vec<u16> {
    const EXTENDED: &[u16] = &[0x5C, 0x5C, 0x3F, 0x5C];
    let wide: Vec<u16> = directory.as_os_str().encode_wide().collect();
    let mut name = if wide.len() >= 8
        && wide.starts_with(EXTENDED)
        && wide[7] == 0x5C
        && String::from_utf16(&wide[4..7]).is_ok_and(|label| label.eq_ignore_ascii_case("UNC"))
    {
        let mut name = vec![0x5C, 0x5C];
        name.extend_from_slice(&wide[8..]);
        name
    } else if wide.starts_with(EXTENDED) {
        wide[4..].to_vec()
    } else {
        wide
    };
    name.push(0);
    name
}

fn platform(error: impl std::fmt::Display) -> FileDialogError {
    FileDialogError::Platform(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_hresult_is_not_a_platform_error() {
        let result = result_from_hresult(9, ERROR_CANCELLED.to_hresult());
        assert!(result.is_cancelled());
        assert_eq!(result.id, 9);
        assert!(result.error.is_none());
    }

    #[test]
    fn other_hresults_are_platform_failures() {
        let result = result_from_hresult(4, HRESULT(0x8000_4005u32 as i32));
        assert!(!result.is_cancelled());
        assert!(matches!(result.error, Some(FileDialogError::Platform(_))));
    }

    #[test]
    fn folder_kinds_set_pickfolders_and_file_kinds_do_not() {
        assert!(dialog_options(FileDialogKind::PickFolder).contains(FOS_PICKFOLDERS));
        assert!(dialog_options(FileDialogKind::PickFolders).contains(FOS_PICKFOLDERS));
        assert!(dialog_options(FileDialogKind::PickFolders).contains(FOS_ALLOWMULTISELECT));
        assert!(!dialog_options(FileDialogKind::OpenFile).contains(FOS_PICKFOLDERS));
        assert!(!dialog_options(FileDialogKind::SaveFile).contains(FOS_PICKFOLDERS));
        assert!(dialog_options(FileDialogKind::OpenFiles).contains(FOS_ALLOWMULTISELECT));
    }

    #[test]
    fn configured_pick_folder_reports_fos_pickfolders() {
        let request = FileDialogRequest::new(1, FileDialogKind::PickFolder)
            .title("选择目录")
            .filters([FileFilter::new("文本", ["txt"])])
            .directory(std::env::temp_dir());
        let configured = inspect(&request).expect("COM dialog can be configured");
        assert!(configured.options.contains(FOS_PICKFOLDERS));
        assert!(configured.extensions.is_empty());
        assert_eq!(configured.title.as_deref(), Some("选择目录"));
        let expected =
            std::fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| std::env::temp_dir());
        let directory = configured
            .directory
            .as_deref()
            .map(PathBuf::from)
            .and_then(|path| std::fs::canonicalize(&path).ok().or(Some(path)));
        assert_eq!(directory, Some(expected));
    }

    #[test]
    fn missing_directory_is_skipped_not_cancelled() {
        let missing = std::env::temp_dir().join("nana-ui-missing-dir-issue34");
        let _ = std::fs::remove_dir_all(&missing);
        let request = FileDialogRequest::new(2, FileDialogKind::PickFolder).directory(&missing);
        let configured = inspect(&request).expect("invalid start folder does not fail configure");
        assert!(configured.options.contains(FOS_PICKFOLDERS));
        if let Some(directory) = configured.directory {
            assert_ne!(PathBuf::from(&directory), missing);
        }
    }

    #[test]
    fn save_dialog_keeps_noreadonlyreturn() {
        let request = FileDialogRequest::new(4, FileDialogKind::SaveFile);
        let configured = inspect(&request).expect("save dialog can be configured");
        assert!(configured.options.contains(FOS_NOREADONLYRETURN));
        assert!(!configured.options.contains(FOS_PICKFOLDERS));
    }

    #[test]
    fn parsing_name_strips_extended_and_unc_prefixes() {
        fn as_os(wide: &[u16]) -> std::ffi::OsString {
            std::ffi::OsString::from_wide(&wide[..wide.len() - 1])
        }
        assert_eq!(
            as_os(&win32_parsing_name(Path::new(r"\\?\C:\Windows"))),
            std::ffi::OsString::from(r"C:\Windows")
        );
        assert_eq!(
            as_os(&win32_parsing_name(Path::new(r"\\?\UNC\server\share"))),
            std::ffi::OsString::from(r"\\server\share")
        );
        assert_eq!(
            as_os(&win32_parsing_name(Path::new(r"\\?\unc\server\share"))),
            std::ffi::OsString::from(r"\\server\share")
        );
        assert_eq!(
            as_os(&win32_parsing_name(Path::new(r"C:\Windows"))),
            std::ffi::OsString::from(r"C:\Windows")
        );
    }
}
