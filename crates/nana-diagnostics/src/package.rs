//! Diagnostic package: a directory a user can zip and attach to a bug
//! report. Archive format and upload are application policy.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::export::json_str;
use crate::export::{ExportOptions, to_json_lines, to_text};
use crate::files::{create_unique, file_stamp, is_session_file, sanitize};
use crate::nlog::read_file;
use crate::runtime::Diagnostics;
use crate::session::unix_now_ns;

#[derive(Debug, Clone)]
pub struct PackageOptions {
    /// Most recent sessions to include (all rotations of each).
    pub max_sessions: usize,
    /// Most recent crash snapshots to include.
    pub max_snapshots: usize,
    /// Copy the binary `.nlog` files. They are not redacted.
    pub include_raw: bool,
    /// Redact the home directory in the text / JSON exports.
    pub redact_home: bool,
}

impl Default for PackageOptions {
    fn default() -> Self {
        Self {
            max_sessions: 3,
            max_snapshots: 5,
            include_raw: true,
            redact_home: true,
        }
    }
}

/// A session log and its rotations share one key: the name without `.nlog`
/// and without a trailing `.{n}` rotation index. Snapshots are keyed by
/// their full name (each is complete on its own).
fn session_key(name: &str) -> &str {
    let base = name.strip_suffix(".nlog").unwrap_or(name);
    match base.rsplit_once('.') {
        Some((head, index)) if !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) => {
            head
        }
        _ => base,
    }
}

/// Files of the `limit` most recently modified sessions (a session counts
/// once however many rotations it has), newest session first.
fn newest(dir: Option<&Path>, app_id: &str, limit: usize, group_rotations: bool) -> Vec<PathBuf> {
    let Some(Ok(read)) = dir.map(fs::read_dir) else {
        return Vec::new();
    };
    let app = sanitize(app_id);
    let mut files: Vec<(SystemTime, PathBuf)> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !is_session_file(name, &app) {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    let mut sessions: Vec<String> = Vec::new();
    let mut picked = Vec::new();
    for (_, path) in files {
        let Some(key) = path.file_name().and_then(|n| n.to_str()).map(|name| {
            if group_rotations {
                session_key(name)
            } else {
                name
            }
        }) else {
            continue;
        };
        if !sessions.iter().any(|s| s == key) {
            if sessions.len() == limit {
                continue;
            }
            sessions.push(key.to_owned());
        }
        picked.push(path);
    }
    picked
}

/// Build a package from log and crash directories without a live runtime
/// (e.g. from a separate "report a problem" tool).
pub fn export_package_from(
    app_id: &str,
    logs: Option<&Path>,
    crash: Option<&Path>,
    dest: &Path,
    options: &PackageOptions,
) -> io::Result<PathBuf> {
    // Never merge into (and overwrite) an earlier export from the same second.
    fs::create_dir_all(dest)?;
    let base = format!(
        "{}-diagnostics-{}",
        sanitize(app_id),
        file_stamp(unix_now_ns())
    );
    let ((), root, _) = create_unique(
        |n| match n {
            0 => dest.join(&base),
            n => dest.join(format!("{base}-{n}")),
        },
        |path| fs::create_dir(path),
    )?;
    let export = ExportOptions {
        redact_home: options.redact_home,
    };
    let mut manifest = String::from("{\n  \"app_id\": ");
    json_str(&mut manifest, app_id);
    manifest.push_str(",\n  \"files\": [");
    let mut first = true;
    for (kind, dir, limit) in [
        ("session", logs, options.max_sessions),
        ("snapshot", crash, options.max_snapshots),
    ] {
        for path in newest(dir, app_id, limit, kind == "session") {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let sub = root.join(kind);
            fs::create_dir_all(&sub)?;
            if options.include_raw {
                fs::copy(&path, sub.join(format!("{stem}.nlog")))?;
            }
            let truncated = match read_file(&path) {
                Ok(file) => {
                    fs::write(sub.join(format!("{stem}.txt")), to_text(&file, &export))?;
                    fs::write(
                        sub.join(format!("{stem}.jsonl")),
                        to_json_lines(&file, &export),
                    )?;
                    Some(file.truncated)
                }
                Err(_) => None,
            };
            manifest.push_str(if first { "\n    " } else { ",\n    " });
            first = false;
            manifest.push_str("{\"kind\": ");
            json_str(&mut manifest, kind);
            manifest.push_str(", \"name\": ");
            json_str(&mut manifest, stem);
            manifest.push_str(match truncated {
                Some(true) => ", \"readable\": true, \"truncated\": true}",
                Some(false) => ", \"readable\": true, \"truncated\": false}",
                None => ", \"readable\": false}",
            });
        }
    }
    manifest.push_str("\n  ]\n}\n");
    fs::write(root.join("manifest.json"), manifest)?;
    Ok(root)
}

impl Diagnostics {
    /// Flush the session log, take a fresh snapshot, and assemble a package
    /// directory under `dest`. Blocks; call from a UI action, not a frame.
    pub fn export_package(&self, dest: &Path, options: &PackageOptions) -> io::Result<PathBuf> {
        let timeout = Duration::from_secs(5);
        self.flush(true, timeout);
        // A package without a fresh snapshot is still useful.
        let _ = self.snapshot_blocking("export", timeout);
        let paths = self.paths();
        export_package_from(
            &self.metadata().app_id,
            paths.logs.as_deref(),
            paths.crash.as_deref(),
            dest,
            options,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::session_key;

    #[test]
    fn rotations_share_their_session_key() {
        assert_eq!(
            session_key("dev.app-20260922T010203Z-7.nlog"),
            "dev.app-20260922T010203Z-7"
        );
        assert_eq!(
            session_key("dev.app-20260922T010203Z-7.3.nlog"),
            "dev.app-20260922T010203Z-7"
        );
        assert_eq!(
            session_key("dev.app-20260922T010203Z-7-panic.nlog"),
            "dev.app-20260922T010203Z-7-panic"
        );
    }
}
