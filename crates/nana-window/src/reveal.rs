//! Show a path in the system file manager.
//!
//! Unlike the file dialog this needs no parent window: the file manager is
//! another process. The call returns once the request is handed off and never
//! waits for the file manager itself.

use std::ffi::OsString;
use std::path::Path;

/// Why a path could not be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevealError {
    /// The path does not exist.
    NotFound,
    /// No file manager could be asked: this target has none, or starting it
    /// failed.
    Unavailable(String),
}

impl std::fmt::Display for RevealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("path does not exist"),
            Self::Unavailable(reason) => write!(f, "file manager unavailable: {reason}"),
        }
    }
}

impl std::error::Error for RevealError {}

/// Shows `path` in the system file manager: a file is shown selected in its
/// folder, a folder is opened.
///
/// | Platform | How |
/// | --- | --- |
/// | macOS | `open -R` selects a file (or a bundle-like folder); `open` opens a folder |
/// | Windows | `explorer.exe /select,` selects a file; `explorer.exe` opens a folder |
/// | Linux | `org.freedesktop.FileManager1` `ShowItems` / `ShowFolders`, falling back to `xdg-open` on the folder |
///
/// Does not block on the file manager. On Linux the D-Bus call and its
/// fallback run on a worker, so a failure there is not reported back.
pub fn reveal_in_file_manager(path: &Path) -> Result<(), RevealError> {
    let target = classify(path)?;
    launch(path, target)
}

/// What the path is, decided before anything is launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    File,
    Folder,
}

fn classify(path: &Path) -> Result<Target, RevealError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(Target::Folder),
        Ok(_) => Ok(Target::File),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(RevealError::NotFound),
        Err(error) => Err(RevealError::Unavailable(error.to_string())),
    }
}

/// The process that shows `path`.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RevealCommand {
    program: &'static str,
    /// On Windows these are passed raw: explorer parses its own command line.
    args: Vec<OsString>,
}

#[cfg(target_os = "macos")]
fn reveal_command(path: &Path, target: Target) -> RevealCommand {
    // A bundle (`.app`, `.bundle`, a document package) is a directory that
    // `open` would launch or hand to an application instead of showing it,
    // so a folder with an extension is selected in its parent instead.
    let select = target == Target::File || path.extension().is_some();
    let mut args = Vec::new();
    if select {
        args.push("-R".into());
    }
    args.push(path.as_os_str().to_owned());
    RevealCommand {
        program: "/usr/bin/open",
        args,
    }
}

#[cfg(target_os = "windows")]
fn reveal_command(path: &Path, target: Target) -> RevealCommand {
    // Explorer wants an absolute path with backslashes and no `\\?\` prefix.
    // Windows paths cannot contain `"`, so quoting is enough.
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let path = explorer_path(&absolute);
    let arg = match target {
        Target::File => format!("/select,\"{path}\""),
        Target::Folder => format!("\"{path}\""),
    };
    RevealCommand {
        program: "explorer.exe",
        args: vec![arg.into()],
    }
}

#[cfg(target_os = "windows")]
fn explorer_path(path: &Path) -> String {
    let path = path.to_string_lossy().replace('/', "\\");
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        path
    }
}

/// The fallback when no FileManager1 service answers: `xdg-open` cannot
/// select, so a file shows its folder.
#[cfg(target_os = "linux")]
fn reveal_command(path: &Path, target: Target) -> RevealCommand {
    let folder = match target {
        Target::Folder => path,
        Target::File => match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        },
    };
    RevealCommand {
        program: "xdg-open",
        args: vec![folder.as_os_str().to_owned()],
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn spawn(command: &RevealCommand) -> Result<(), RevealError> {
    let mut process = std::process::Command::new(command.program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        for arg in &command.args {
            process.raw_arg(arg);
        }
    }
    #[cfg(not(target_os = "windows"))]
    process.args(&command.args);
    process
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = process
        .spawn()
        .map_err(|error| RevealError::Unavailable(error.to_string()))?;
    // The exit status is not the outcome: explorer exits 1 after a successful
    // `/select`. Reap on a worker so a Unix child does not linger as a zombie.
    let _ = std::thread::Builder::new()
        .name("nana-reveal-reap".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn launch(path: &Path, target: Target) -> Result<(), RevealError> {
    spawn(&reveal_command(path, target))
}

#[cfg(target_os = "linux")]
fn launch(path: &Path, target: Target) -> Result<(), RevealError> {
    let absolute =
        std::path::absolute(path).map_err(|error| RevealError::Unavailable(error.to_string()))?;
    let uri = url::Url::from_file_path(&absolute)
        .map_err(|()| RevealError::Unavailable("path has no file URI".into()))?
        .to_string();
    let fallback = reveal_command(&absolute, target);
    std::thread::Builder::new()
        .name("nana-reveal".into())
        .spawn(move || {
            let shown = futures_lite::future::block_on(show_with_file_manager(uri, target));
            if shown.is_err() {
                let _ = spawn(&fallback);
            }
        })
        .map(|_| ())
        .map_err(|error| RevealError::Unavailable(error.to_string()))
}

/// The freedesktop file manager interface (Nautilus, Dolphin, Nemo, Thunar,
/// Caja, …): `ShowItems` selects, `ShowFolders` opens.
#[cfg(target_os = "linux")]
async fn show_with_file_manager(uri: String, target: Target) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let method = match target {
        Target::File => "ShowItems",
        Target::Folder => "ShowFolders",
    };
    connection
        .call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            method,
            &(vec![uri], ""),
        )
        .await?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn launch(_path: &Path, _target: Target) -> Result<(), RevealError> {
    Err(RevealError::Unavailable(
        "no file manager on this target".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nana-reveal-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn a_missing_path_is_not_found_before_anything_launches() {
        let dir = scratch("missing");
        let missing = dir.join("absent.txt");
        assert_eq!(classify(&missing), Err(RevealError::NotFound));
        assert_eq!(reveal_in_file_manager(&missing), Err(RevealError::NotFound));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn files_and_folders_are_told_apart() {
        let dir = scratch("classify");
        let file = dir.join("model.json");
        std::fs::write(&file, b"{}").expect("scratch file");
        assert_eq!(classify(&file), Ok(Target::File));
        assert_eq!(classify(&dir), Ok(Target::Folder));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_selects_files_and_bundles_and_opens_plain_folders() {
        let file = reveal_command(Path::new("/a/b.json"), Target::File);
        assert_eq!(file.program, "/usr/bin/open");
        assert_eq!(file.args, ["-R", "/a/b.json"]);
        let folder = reveal_command(Path::new("/a/models"), Target::Folder);
        assert_eq!(folder.args, ["/a/models"]);
        let bundle = reveal_command(Path::new("/Applications/Nana.app"), Target::Folder);
        assert_eq!(bundle.args, ["-R", "/Applications/Nana.app"]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_selects_files_with_one_raw_quoted_argument() {
        let file = reveal_command(Path::new(r"C:\a b\c.json"), Target::File);
        assert_eq!(file.program, "explorer.exe");
        assert_eq!(file.args, [r#"/select,"C:\a b\c.json""#]);
        let folder = reveal_command(Path::new("C:/a b/models"), Target::Folder);
        assert_eq!(folder.args, [r#""C:\a b\models""#]);
        assert_eq!(explorer_path(Path::new(r"\\?\C:\x")), r"C:\x");
        assert_eq!(
            explorer_path(Path::new(r"\\?\UNC\srv\share")),
            r"\\srv\share"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_fallback_opens_the_folder_holding_a_file() {
        let file = reveal_command(Path::new("/a/b.json"), Target::File);
        assert_eq!(file.program, "xdg-open");
        assert_eq!(file.args, ["/a"]);
        let folder = reveal_command(Path::new("/a/models"), Target::Folder);
        assert_eq!(folder.args, ["/a/models"]);
        assert_eq!(
            reveal_command(Path::new("b.json"), Target::File).args,
            ["."]
        );
    }
}
