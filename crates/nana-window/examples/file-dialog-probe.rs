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

    let request = FileDialogRequest::new(42, FileDialogKind::OpenFile)
        .title("选择图片")
        .filters([
            FileFilter::new("图片", ["png", "jpg"]),
            FileFilter::new("文档", ["pdf"]),
        ])
        .directory("/tmp");

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
    if directory.as_deref() != Some("/tmp") {
        failures.push("starting directory did not reach the panel");
    }
    if extensions != ["png", "jpg", "pdf"] {
        failures.push("filters did not reach the panel");
    }

    if failures.is_empty() {
        println!("OK: the request reached AppKit intact");
        std::process::ExitCode::SUCCESS
    } else {
        for failure in failures {
            eprintln!("FAIL: {failure}");
        }
        std::process::ExitCode::FAILURE
    }
}
