//! Diagnostic package: a directory a user can zip and attach to a bug
//! report. Archive format and upload are application policy.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::export::json_str;
use crate::export::{ExportOptions, to_json_lines, to_text};
use crate::files::{create_unique, file_stamp, is_session_file, sanitize};
use crate::nlog::{NlogFile, read_file};
use crate::runtime::Diagnostics;
use crate::session::unix_now_ns;

/// In-place filter/redaction hook invoked on each decoded [`NlogFile`] before
/// text and JSONL package outputs are written.
pub type PackageFilter = Arc<dyn Fn(&mut NlogFile) + Send + Sync>;

#[derive(Clone)]
pub struct PackageOptions {
    /// Most recent sessions to include (all rotations of each).
    pub max_sessions: usize,
    /// Most recent crash snapshots to include.
    pub max_snapshots: usize,
    /// Copy the binary `.nlog` files. They are not redacted.
    pub include_raw: bool,
    /// Redact the home directory in the text / JSON exports.
    pub redact_home: bool,
    /// Optional per-file filter/redaction callback run on each decoded
    /// [`NlogFile`] before writing `.txt` and `.jsonl` outputs.
    pub filter: Option<PackageFilter>,
}

impl fmt::Debug for PackageOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PackageOptions")
            .field("max_sessions", &self.max_sessions)
            .field("max_snapshots", &self.max_snapshots)
            .field("include_raw", &self.include_raw)
            .field("redact_home", &self.redact_home)
            .field("filter", &self.filter.as_ref().map(|_| "<filter>"))
            .finish()
    }
}

impl Default for PackageOptions {
    fn default() -> Self {
        Self {
            max_sessions: 3,
            max_snapshots: 5,
            include_raw: true,
            redact_home: true,
            filter: None,
        }
    }
}

impl PackageOptions {
    /// Attach an in-place filter/redaction hook invoked once per decoded
    /// `.nlog` file before `.txt` and `.jsonl` outputs are generated.
    pub fn with_filter(mut self, filter: impl Fn(&mut NlogFile) + Send + Sync + 'static) -> Self {
        self.filter = Some(Arc::new(filter));
        self
    }
}

/// Removes an unpublished staging directory if export returns early or panics.
struct StagingGuard(Option<PathBuf>);

impl StagingGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = fs::remove_dir_all(dir);
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
///
/// The package is assembled in a hidden sibling staging directory under `dest`
/// and atomically renamed into place once `manifest.json` is written, so an
/// interrupted or failing export never leaves a partial or unredacted bundle at
/// the destination path.
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
    let pid = std::process::id();
    let ((), staging, _) = create_unique(
        |n| dest.join(format!(".{base}.staging-{pid}-{n}")),
        |path| fs::create_dir(path),
    )?;
    let mut guard = StagingGuard(Some(staging.clone()));
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
            let sub = staging.join(kind);
            fs::create_dir_all(&sub)?;
            if options.include_raw {
                fs::copy(&path, sub.join(format!("{stem}.nlog")))?;
            }
            let truncated = match read_file(&path) {
                Ok(mut file) => {
                    if let Some(filter) = &options.filter {
                        filter(&mut file);
                    }
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
    fs::write(staging.join("manifest.json"), manifest)?;

    for n in 0u32..10_000 {
        let candidate = match n {
            0 => dest.join(&base),
            n => dest.join(format!("{base}-{n}")),
        };
        if candidate.exists() {
            continue;
        }
        match fs::rename(&staging, &candidate) {
            Ok(()) => {
                guard.disarm();
                return Ok(candidate);
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
                ) =>
            {
                continue;
            }
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not pick a unique package directory name",
    ))
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
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::{
        Diagnostics, DiagnosticsConfig, DiagnosticsPaths, PackageOptions, SessionMetadata,
        export_package_from,
    };

    use super::session_key;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(1);

    fn temp_dir(name: &str) -> PathBuf {
        let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("nana-diag-pkg-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

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

    #[test]
    fn package_filter_redacts_in_single_pass_and_leaves_no_staging_dir() {
        let root = temp_dir("filter");
        let logs = root.join("logs");
        let crash = root.join("crash");
        let out = root.join("out");

        let mut meta = SessionMetadata::new("dev.pkg.filter", "PkgFilter", "1.0.0");
        meta.extra
            .push(("secret_token".into(), "top-secret-value".into()));
        let diag = Diagnostics::start(
            DiagnosticsConfig::default(),
            meta,
            DiagnosticsPaths {
                logs: Some(logs.clone()),
                crash: Some(crash.clone()),
            },
        );
        diag.set_session_info("secret_token", "top-secret-value");
        diag.marker("secret-marker-line");
        assert!(diag.flush(true, Duration::from_secs(5)));
        diag.shutdown();

        let filter_calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = filter_calls.clone();
        let options = PackageOptions {
            include_raw: false,
            ..PackageOptions::default()
        }
        .with_filter(move |file| {
            calls_clone.fetch_add(1, Ordering::Relaxed);
            for (k, v) in &mut file.header.extra {
                if k == "secret_token" {
                    *v = "[REDACTED]".into();
                }
            }
            for entry in &mut file.entries {
                match entry {
                    crate::nlog::Entry::SessionInfo { pairs, .. } => {
                        for (k, v) in pairs {
                            if k == "secret_token" {
                                *v = "[REDACTED]".into();
                            }
                        }
                    }
                    crate::nlog::Entry::Marker { text, .. } => {
                        if text.contains("secret") {
                            *text = "[REDACTED]".into();
                        }
                    }
                    _ => {}
                }
            }
        });

        let pkg = export_package_from("dev.pkg.filter", Some(&logs), Some(&crash), &out, &options)
            .expect("export package");

        assert!(filter_calls.load(Ordering::Relaxed) >= 1);
        assert!(pkg.join("manifest.json").is_file());

        // No hidden staging directories left behind in `out`.
        let entries: Vec<String> = fs::read_dir(&out)
            .expect("read out")
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].starts_with('.'));

        let session_dir = pkg.join("session");
        let mut saw_txt = false;
        let mut saw_jsonl = false;
        for entry in fs::read_dir(&session_dir)
            .expect("read session")
            .filter_map(Result::ok)
        {
            let path = entry.path();
            assert_ne!(path.extension().and_then(|e| e.to_str()), Some("nlog"));
            let body = fs::read_to_string(&path).expect("read exported file");
            assert!(
                !body.contains("top-secret-value"),
                "unredacted secret leaked into {}",
                path.display()
            );
            assert!(
                !body.contains("secret-marker-line"),
                "unredacted marker leaked into {}",
                path.display()
            );
            assert!(body.contains("[REDACTED]"));
            match path.extension().and_then(|e| e.to_str()) {
                Some("txt") => saw_txt = true,
                Some("jsonl") => saw_jsonl = true,
                _ => {}
            }
        }
        assert!(saw_txt && saw_jsonl);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_export_cleans_up_staging_directory() {
        let root = temp_dir("staging-cleanup");
        let logs = root.join("logs");
        let out = root.join("out");

        let diag = Diagnostics::start(
            DiagnosticsConfig::default(),
            SessionMetadata::new("dev.pkg.panic", "PkgPanic", "1.0.0"),
            DiagnosticsPaths {
                logs: Some(logs.clone()),
                crash: None,
            },
        );
        diag.marker("hello");
        assert!(diag.flush(true, Duration::from_secs(5)));
        diag.shutdown();

        let options = PackageOptions::default().with_filter(|_| {
            panic!("simulated filter failure");
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = export_package_from("dev.pkg.panic", Some(&logs), None, &out, &options);
        }));
        assert!(result.is_err());

        // Staging directory must have been cleaned up and no partial bundle created.
        let leftover: Vec<_> = fs::read_dir(&out)
            .expect("read out")
            .filter_map(Result::ok)
            .collect();
        assert!(leftover.is_empty(), "leftover entries in out: {leftover:?}");

        let _ = fs::remove_dir_all(&root);
    }
}
