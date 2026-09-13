//! Verifies a file dialog request reaches the platform intact.
//!
//! A file dialog is modal and cannot be driven from a shell, so this checks
//! the half that is ours: build and configure the real platform panel from a
//! request, then read back what it holds. Runs on the main thread, which
//! AppKit requires.

use nana_window::{
    FileDialogKind, FileDialogRequest, FileFilter, describe_configured_dialog, file_dialog_support,
};

fn main() -> std::process::ExitCode {
    println!("support: {:?}", file_dialog_support());

    let start = std::env::temp_dir();
    let request = FileDialogRequest::new(42, FileDialogKind::OpenFile)
        .title("选择图片")
        .filters([
            FileFilter::new("图片", ["png", "jpg"]),
            FileFilter::new("文档", ["pdf"]),
        ])
        .directory(start.clone());

    let Some((title, directory, extensions)) = describe_configured_dialog(&request) else {
        println!("platform cannot be queried here; nothing to verify");
        return std::process::ExitCode::SUCCESS;
    };
    println!("title: {title:?}");
    println!("directory: {directory:?}");
    println!("extensions: {extensions:?}");

    let mut failures = Vec::new();
    if title.as_deref() != Some("选择图片") {
        failures.push("title did not reach the panel");
    }
    let expected = std::fs::canonicalize(&start).unwrap_or(start);
    if !same_path(directory.as_deref(), &expected) {
        failures.push("starting directory did not reach the panel");
    }
    if extensions != ["png", "jpg", "pdf"] {
        failures.push("filters did not reach the panel");
    }

    let folder = FileDialogRequest::new(43, FileDialogKind::PickFolder)
        .title("选择目录")
        .filters([FileFilter::new("文本", ["txt"])])
        .directory(expected.clone());
    match describe_configured_dialog(&folder) {
        Some((title, directory, extensions)) => {
            if title.as_deref() != Some("选择目录") {
                failures.push("folder title did not reach the panel");
            }
            if !same_path(directory.as_deref(), &expected) {
                failures.push("folder starting directory did not reach the panel");
            }
            // IFileDialog rejects SetFileTypes with FOS_PICKFOLDERS; AppKit still
            // records allowedFileTypes on a directory panel.
            if cfg!(target_os = "windows") {
                if !extensions.is_empty() {
                    failures.push("folder dialog should not apply file filters");
                }
            } else if extensions != ["txt"] {
                failures.push("folder filters did not reach the panel");
            }
        }
        None => failures.push("folder request could not be queried"),
    }

    if failures.is_empty() {
        println!("OK: the request reached the platform intact");
        std::process::ExitCode::SUCCESS
    } else {
        for failure in failures {
            eprintln!("FAIL: {failure}");
        }
        std::process::ExitCode::FAILURE
    }
}

fn same_path(got: Option<&str>, expected: &std::path::Path) -> bool {
    got.map(std::path::PathBuf::from)
        .and_then(|path| std::fs::canonicalize(&path).ok().or(Some(path)))
        .as_deref()
        == Some(expected)
}
