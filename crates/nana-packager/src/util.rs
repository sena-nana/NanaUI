//! Small helpers every packager step shares.

use std::path::{Path, PathBuf};

/// Every file under `root`, sorted. Directories are descended without
/// following symlinks; a symlink to a file counts as that file.
pub(crate) fn walk_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = std::fs::read_dir(&dir)
            .map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
        for item in read {
            let item = item.map_err(|error| error.to_string())?;
            let path = item.path();
            let kind = item.file_type().map_err(|error| error.to_string())?;
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() || path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// `path` relative to `root`, `/`-separated.
pub(crate) fn relative_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("{} is outside {}", path.display(), root.display()))?;
    relative
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("/"))
        .ok_or_else(|| format!("{} is not UTF-8", relative.display()))
}

/// Hex BLAKE3 of `bytes`, as manifests and Steam file lists record it.
pub(crate) fn content_hex(bytes: &[u8]) -> String {
    nana_package::to_hex(&nana_package::hash::content(bytes))
}

pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

pub(crate) fn io(context: &'static str) -> impl Fn(std::io::Error) -> String {
    move |error| format!("{context}: {error}")
}
