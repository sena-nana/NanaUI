//! Session-log output: file naming, rotation, retention, and the pluggable
//! [`Sink`]. Runs on the worker thread only.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::session::Retention;

/// Destination for the session log byte stream. The first `write` of a
/// stream starts with the file header.
pub trait Sink: Send {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// `durable`: also ask the OS to put the bytes on disk (`fsync`-class).
    fn flush(&mut self, durable: bool) -> io::Result<()>;
}

/// `2026-09-22T10:15:30Z` → `20260922T101530Z`.
pub(crate) fn file_stamp(unix_ns: u64) -> String {
    let secs = unix_ns / 1_000_000_000;
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// ISO-8601 UTC with milliseconds, for text export.
pub(crate) fn iso8601(unix_ns: u64) -> String {
    let secs = unix_ns / 1_000_000_000;
    let millis = (unix_ns / 1_000_000) % 1000;
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// Howard Hinnant's days-from-civil inverse.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Keep file names portable: anything outside `[A-Za-z0-9._-]` becomes `_`.
pub(crate) fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "app".into()
    } else {
        cleaned
    }
}

/// `{app}-{stamp}-{pid}` — shared by the session log and its snapshots.
pub(crate) fn session_stem(app_id: &str, wall_start_unix_ns: u64, pid: u32) -> String {
    format!(
        "{}-{}-{pid}",
        sanitize(app_id),
        file_stamp(wall_start_unix_ns)
    )
}

/// Whether `name` is one of `app_id`'s session logs or snapshots:
/// `{app}-{YYYYMMDD}T{HHMMSS}Z-{pid}…​.nlog`. A plain `{app}-` prefix match
/// would also claim `{app}-beta-…`, another app's files.
pub(crate) fn is_session_file(name: &str, app_id: &str) -> bool {
    let app = sanitize(app_id);
    let Some(rest) = name
        .strip_prefix(app.as_str())
        .and_then(|rest| rest.strip_prefix('-'))
    else {
        return false;
    };
    let stamp = rest.as_bytes();
    name.ends_with(".nlog")
        && stamp.len() > 17
        && stamp[..8].iter().all(u8::is_ascii_digit)
        && stamp[8] == b'T'
        && stamp[9..15].iter().all(u8::is_ascii_digit)
        && stamp[15] == b'Z'
        && stamp[16] == b'-'
        && stamp[17].is_ascii_digit()
}

/// Write `bytes` to a new file in `dir` and put it on disk.
pub(crate) fn write_new_file(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let mut path = dir.join(format!("{name}.nlog"));
    let mut attempt = 1;
    let mut file = loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => break file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => {
                path = dir.join(format!("{name}-{attempt}.nlog"));
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    };
    file.write_all(bytes)?;
    file.sync_data()?;
    Ok(path)
}

/// Whether `name` belongs to the session whose stem is `stem`: the stem
/// followed by `.` (the log and its rotations) or `-` (its snapshots). A bare
/// prefix match would let pid 12 claim pid 123's files.
fn is_own_file(name: &str, stem: &str) -> bool {
    name.strip_prefix(stem)
        .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('-'))
}

/// Delete `app_id`'s oldest `.nlog` files under `dir` until the retention
/// limits hold. Never touches `keep`. Files modified within
/// `retention.live_grace` are spared only when they belong to *another*
/// session (`own_stem` names this one): that is what protects a second
/// running instance, and it must not let this session's own rotations or
/// snapshots grow without bound.
pub(crate) fn prune(
    dir: &Path,
    app_id: &str,
    own_stem: Option<&str>,
    keep: Option<&Path>,
    max_files: usize,
    max_total_bytes: u64,
    retention: &Retention,
) -> usize {
    let Ok(read) = fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut files: Vec<(PathBuf, SystemTime, u64)> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !is_session_file(name, app_id) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            meta.is_file()
                .then(|| (path, meta.modified().unwrap_or(now), meta.len()))
        })
        .collect();
    files.sort_by_key(|(_, modified, _)| *modified);
    let mut count = files.len();
    let mut total: u64 = files.iter().map(|(_, _, len)| len).sum();
    let mut removed = 0;
    for (path, modified, len) in files {
        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        let over = count > max_files || total > max_total_bytes || age > retention.max_age;
        if !over {
            // Oldest first: once one file is within limits, the rest are too,
            // except for age, which is monotonic in this order as well.
            break;
        }
        let own = own_stem.is_some_and(|stem| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| is_own_file(name, stem))
        });
        if keep == Some(path.as_path()) || (age < retention.live_grace && !own) {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            count -= 1;
            total = total.saturating_sub(len);
            removed += 1;
        }
    }
    removed
}

/// Rotating session log in a directory.
pub(crate) struct RotatingLog {
    dir: PathBuf,
    /// Pruning matches every session of this app, not just this one.
    app_id: String,
    stem: String,
    retention: Retention,
    file: Option<(File, PathBuf)>,
    written: u64,
    index: u32,
}

impl RotatingLog {
    pub(crate) fn new(dir: PathBuf, app_id: &str, stem: String, retention: Retention) -> Self {
        Self {
            dir,
            app_id: app_id.to_owned(),
            stem,
            retention,
            file: None,
            written: 0,
            index: 0,
        }
    }

    pub(crate) fn current_path(&self) -> Option<&Path> {
        self.file.as_ref().map(|(_, path)| path.as_path())
    }

    /// A new file must start with the header and all schemas seen so far.
    pub(crate) fn needs_prefix(&self, incoming: usize) -> bool {
        self.file.is_none()
            || (self.written > 0 && self.written + incoming as u64 > self.retention.max_file_bytes)
    }

    /// Drop the current file; the next write reopens with a prefix.
    pub(crate) fn reset(&mut self) {
        self.file = None;
    }

    /// `prefix` is required exactly when [`Self::needs_prefix`] said so.
    pub(crate) fn write(&mut self, prefix: Option<&[u8]>, batch: &[u8]) -> io::Result<()> {
        if let Some(prefix) = prefix {
            self.rotate()?;
            let mut first = Vec::with_capacity(prefix.len() + batch.len());
            first.extend_from_slice(prefix);
            first.extend_from_slice(batch);
            return self.append(&first);
        }
        self.append(batch)
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let Some((file, _)) = self.file.as_mut() else {
            return Err(io::Error::other("session log is not open"));
        };
        file.write_all(bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        if let Some((mut file, _)) = self.file.take() {
            let _ = file.flush();
        }
        fs::create_dir_all(&self.dir)?;
        // Never truncate: a same-second restart of the same pid (or a stale
        // file) must not lose an earlier session's log.
        let (file, path) = loop {
            let name = match self.index {
                0 => format!("{}.nlog", self.stem),
                n => format!("{}.{n}.nlog", self.stem),
            };
            self.index += 1;
            let path = self.dir.join(name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break (file, path),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && self.index < 10_000 => {}
                Err(e) => return Err(e),
            }
        };
        self.file = Some((file, path));
        self.written = 0;
        self.prune();
        Ok(())
    }

    pub(crate) fn prune(&self) -> usize {
        prune(
            &self.dir,
            &self.app_id,
            Some(&self.stem),
            self.current_path(),
            self.retention.max_files,
            self.retention.max_total_bytes,
            &self.retention,
        )
    }

    pub(crate) fn flush(&mut self, durable: bool) -> io::Result<()> {
        if let Some((file, _)) = self.file.as_mut() {
            file.flush()?;
            if durable {
                file.sync_data()?;
            }
        }
        Ok(())
    }
}

/// Retry schedule after a failed write: the batch is discarded (memory stays
/// bounded) and writing resumes with a fresh file after the delay.
pub(crate) struct Backoff {
    until: Option<Instant>,
    delay: Duration,
}

impl Backoff {
    const FIRST: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(60);

    pub(crate) fn new() -> Self {
        Self {
            until: None,
            delay: Self::FIRST,
        }
    }

    pub(crate) fn blocked(&self, now: Instant) -> bool {
        self.until.is_some_and(|until| now < until)
    }

    pub(crate) fn failed(&mut self, now: Instant) {
        self.until = Some(now + self.delay);
        self.delay = (self.delay * 2).min(Self::MAX);
    }

    pub(crate) fn succeeded(&mut self) {
        self.until = None;
        self.delay = Self::FIRST;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_round_trip_known_points() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        // 2026-09-22 is day 20_718.
        assert_eq!(civil_from_days(20_718), (2026, 9, 22));
        assert_eq!(
            file_stamp(20_718 * 86_400_000_000_000 + 3_723_000_000_000),
            "20260922T010203Z"
        );
        assert_eq!(iso8601(1_500_000_000), "1970-01-01T00:00:01.500Z");
    }

    #[test]
    fn session_files_match_the_exact_stem_only() {
        assert!(is_session_file(
            "dev.nana-20260922T010203Z-42.nlog",
            "dev.nana"
        ));
        assert!(is_session_file(
            "dev.nana-20260922T010203Z-42.3.nlog",
            "dev.nana"
        ));
        assert!(is_session_file(
            "dev.nana-20260922T010203Z-42-panic.nlog",
            "dev.nana"
        ));
        assert!(!is_session_file(
            "dev.nana-beta-20260922T010203Z-42.nlog",
            "dev.nana"
        ));
        assert!(!is_session_file(
            "dev.nana-20260922T010203Z-42.txt",
            "dev.nana"
        ));
        assert!(!is_session_file(
            "dev.nanax-20260922T010203Z-42.nlog",
            "dev.nana"
        ));
    }

    #[test]
    fn own_files_need_a_separator_after_the_stem() {
        let stem = "app-20260922T010203Z-12";
        assert!(is_own_file("app-20260922T010203Z-12.nlog", stem));
        assert!(is_own_file("app-20260922T010203Z-12.3.nlog", stem));
        assert!(is_own_file("app-20260922T010203Z-12-panic.nlog", stem));
        assert!(!is_own_file("app-20260922T010203Z-123.nlog", stem));
    }

    #[test]
    fn sanitize_keeps_names_portable() {
        assert_eq!(sanitize("dev.nana/live app"), "dev.nana_live_app");
        assert_eq!(sanitize(""), "app");
    }
}
