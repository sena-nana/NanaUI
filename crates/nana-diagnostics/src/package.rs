//! Diagnostic package: a directory a user can zip and attach to a bug
//! report. Archive format and upload are application policy.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::export::{ExportOptions, to_json_lines, to_text};
use crate::files::{file_stamp, is_session_file, sanitize};
use crate::nlog::read_file;
use crate::runtime::Diagnostics;
use crate::session::unix_now_ns;

#[derive(Debug, Clone)]
pub struct PackageOptions {
    /// Most recent session logs to include.
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

fn newest(dir: Option<&Path>, app_id: &str, limit: usize) -> Vec<PathBuf> {
    let Some(Ok(read)) = dir.map(fs::read_dir) else {
        return Vec::new();
    };
    let mut files: Vec<(SystemTime, PathBuf)> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !is_session_file(name, app_id) {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    files.into_iter().take(limit).map(|(_, p)| p).collect()
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
    let root = dest.join(format!(
        "{}-diagnostics-{}",
        sanitize(app_id),
        file_stamp(unix_now_ns())
    ));
    fs::create_dir_all(&root)?;
    let export = ExportOptions {
        redact_home: options.redact_home,
    };
    let mut manifest = String::from("{\n  \"app_id\": ");
    push_json_str(&mut manifest, app_id);
    manifest.push_str(",\n  \"files\": [");
    let mut first = true;
    for (kind, dir, limit) in [
        ("session", logs, options.max_sessions),
        ("snapshot", crash, options.max_snapshots),
    ] {
        for path in newest(dir, app_id, limit) {
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
            push_json_str(&mut manifest, kind);
            manifest.push_str(", \"name\": ");
            push_json_str(&mut manifest, stem);
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

fn push_json_str(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
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
