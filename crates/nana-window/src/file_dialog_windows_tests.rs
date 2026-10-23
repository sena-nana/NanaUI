//! Windows 系统文件对话框配置、取消结果与路径规范化回归。
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

fn folder_path(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(PathBuf::from)
        .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
}

#[test]
fn missing_directory_is_skipped_not_cancelled() {
    let missing = std::env::temp_dir().join("nana-ui-missing-dir-issue34");
    let _ = std::fs::remove_dir_all(&missing);
    let baseline = inspect(&FileDialogRequest::new(2, FileDialogKind::PickFolder))
        .expect("default folder dialog can be configured");
    let configured =
        inspect(&FileDialogRequest::new(2, FileDialogKind::PickFolder).directory(&missing))
            .expect("invalid start folder does not fail configure");
    assert!(configured.options.contains(FOS_PICKFOLDERS));
    assert_eq!(
        folder_path(configured.directory.as_deref()),
        folder_path(baseline.directory.as_deref())
    );
    if let Some(directory) = configured.directory {
        let got = folder_path(Some(&directory)).expect("configured folder");
        assert_ne!(got, missing);
        assert_ne!(got, std::fs::canonicalize(&missing).unwrap_or(missing));
    }
}

#[test]
fn file_path_as_directory_is_skipped() {
    let file = std::env::temp_dir().join("nana-ui-not-a-dir-issue34.txt");
    std::fs::write(&file, b"x").expect("temp file for directory fallback");
    let baseline = inspect(&FileDialogRequest::new(3, FileDialogKind::PickFolder))
        .expect("default folder dialog can be configured");
    let configured =
        inspect(&FileDialogRequest::new(3, FileDialogKind::PickFolder).directory(&file));
    let _ = std::fs::remove_file(&file);
    let configured = configured.expect("file start folder does not fail configure");
    assert!(configured.options.contains(FOS_PICKFOLDERS));
    assert_eq!(
        folder_path(configured.directory.as_deref()),
        folder_path(baseline.directory.as_deref())
    );
}

#[test]
fn folder_results_reject_existing_files() {
    let file = std::env::temp_dir().join("nana-ui-folder-result-issue34.txt");
    std::fs::write(&file, b"x").expect("temp file for folder result");
    let error = require_directories(FileDialogKind::PickFolder, vec![file.clone()])
        .expect_err("existing file is not a folder result");
    let _ = std::fs::remove_file(&file);
    assert!(matches!(error, FileDialogError::Platform(_)));
    let dir = std::env::temp_dir();
    let paths = require_directories(FileDialogKind::PickFolder, vec![dir.clone()]).expect("dir ok");
    assert_eq!(paths, vec![dir]);
    let file_kind = require_directories(
        FileDialogKind::OpenFile,
        vec![PathBuf::from("not-checked.txt")],
    )
    .expect("file kinds keep their paths");
    assert_eq!(file_kind, vec![PathBuf::from("not-checked.txt")]);
}

#[test]
fn folder_kind_ignores_file_name() {
    let request = FileDialogRequest::new(5, FileDialogKind::PickFolder)
        .file_name("nana-dialog-selection.txt")
        .directory(std::env::temp_dir());
    let configured = inspect(&request).expect("folder dialog ignores save file name");
    assert!(configured.options.contains(FOS_PICKFOLDERS));
    assert_eq!(configured.file_name, None);
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
    for raw in [
        r"\\?\C:\name.",
        r"\\?\UNC\server\share\name ",
        r"\\?\Volume{1234}\x",
    ] {
        assert_eq!(
            as_os(&win32_parsing_name(Path::new(raw))),
            std::ffi::OsString::from(raw)
        );
    }
}
