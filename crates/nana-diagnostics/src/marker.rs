//! The run marker: proof that a session ended cleanly.
//!
//! Each session creates `{stem}.running` in the crash directory, holds it
//! locked for its lifetime, rewrites its last-alive time on every metric
//! snapshot, and deletes it on shutdown. A marker that a new session can lock
//! belongs to a process that is gone without shutting down: killed, powered
//! off, or crashed in a way nothing in-process could record. The operating
//! system releases the lock when the process dies, so a live instance's
//! marker is never mistaken for a stale one and pid reuse does not matter.
//!
//! Writing the marker never allocates after [`RunMarker::create`], so the
//! native crash writer can leave its breadcrumb (exception code and address)
//! in it even when the heap is unusable.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::files::{is_session_file, sanitize};
use crate::session::SessionMetadata;

pub(crate) const EXTENSION: &str = "running";

/// Text formatted into a fixed buffer: never allocates, truncates on
/// overflow.
pub(crate) struct FixedText<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FixedText<N> {
    pub(crate) const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl<const N: usize> fmt::Write for FixedText<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let take = s.len().min(N - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

/// This session's marker, open and locked.
pub(crate) struct RunMarker {
    file: File,
    path: PathBuf,
    /// The lines that never change, formatted once.
    header: String,
    last_unix_ns: u64,
    exception: Option<(u64, u64)>,
}

impl RunMarker {
    pub(crate) fn create(dir: &Path, stem: &str, meta: &SessionMetadata) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(format!("{stem}.{EXTENSION}"));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        // Another live instance with the same stem (same app, second and
        // pid: only tests) keeps its marker.
        file.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => {
                io::Error::new(io::ErrorKind::WouldBlock, "run marker is held")
            }
            fs::TryLockError::Error(e) => e,
        })?;
        let header = format!(
            "nana-run-marker=1\nsession_id={}\npid={}\nstart_unix_ns={}\napp_version={}\n",
            meta.session_id,
            meta.pid,
            meta.wall_start_unix_ns,
            one_line(&meta.app_version),
        );
        let mut marker = Self {
            file,
            path,
            header,
            last_unix_ns: meta.wall_start_unix_ns,
            exception: None,
        };
        marker.write()?;
        marker.file.sync_data()?;
        Ok(marker)
    }

    fn write(&mut self) -> io::Result<()> {
        use fmt::Write as _;
        let mut tail = FixedText::<160>::new();
        let _ = writeln!(tail, "last_unix_ns={}", self.last_unix_ns);
        if let Some((code, address)) = self.exception {
            let _ = write!(
                tail,
                "exception_code=0x{code:x}\nexception_address=0x{address:x}\n"
            );
        }
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(self.header.as_bytes())?;
        self.file.write_all(tail.as_bytes())?;
        self.file
            .set_len((self.header.len() + tail.as_bytes().len()) as u64)
    }

    /// The session is still alive at `unix_ns`. Not synced: losing the last
    /// few seconds of this to a power cut is fine.
    pub(crate) fn heartbeat(&mut self, unix_ns: u64) {
        self.last_unix_ns = unix_ns;
        let _ = self.write();
    }

    /// Leave a native exception in the marker and put it on disk. Allocation
    /// free.
    pub(crate) fn record_exception(&mut self, code: u64, address: u64, unix_ns: u64) {
        self.last_unix_ns = unix_ns;
        self.exception = Some((code, address));
        if self.write().is_ok() {
            let _ = self.file.sync_data();
        }
    }

    /// Clean shutdown: the marker goes away.
    pub(crate) fn remove(self) {
        // Deleted while still open and locked, so no other process can lock
        // the file between the unlock and the delete and report this session.
        let _ = fs::remove_file(&self.path);
    }
}

impl UncleanExit {
    /// One line for the fault message.
    pub fn describe(&self) -> String {
        use fmt::Write as _;
        let time = |ns: u64| {
            if ns == 0 {
                "unknown".to_owned()
            } else {
                crate::files::iso8601(ns)
            }
        };
        let version = if self.app_version.is_empty() {
            "unknown"
        } else {
            &self.app_version
        };
        let mut text = format!(
            "previous session {:016x} (version {version}, pid {}) started {}, last alive {}, did not shut down",
            self.session_id,
            self.pid,
            time(self.start_unix_ns),
            time(self.last_write_unix_ns),
        );
        if let Some((code, address)) = self.exception {
            let _ = write!(text, "; native exception 0x{code:08x} at 0x{address:x}");
        }
        match self.crash_snapshot.as_ref().and_then(|p| p.file_name()) {
            Some(name) => {
                let _ = write!(text, "; crash snapshot {}", name.to_string_lossy());
            }
            None => text.push_str("; no crash snapshot"),
        }
        text
    }
}

fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(128)
        .collect()
}

/// An earlier session of this app that ended without shutting down: its run
/// marker was still there when this session started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncleanExit {
    /// `{app}-{stamp}-{pid}`: the name its logs and snapshots start with.
    pub stem: String,
    pub session_id: u64,
    pub pid: u64,
    pub start_unix_ns: u64,
    /// The latest sign of life: the marker's own heartbeat (every metric
    /// interval) or the newest of the session's files.
    pub last_write_unix_ns: u64,
    pub app_version: String,
    /// Native exception code and address the crash writer left, if it got
    /// that far.
    pub exception: Option<(u64, u64)>,
    /// A panic or native-crash snapshot the session wrote before it died.
    pub crash_snapshot: Option<PathBuf>,
}

/// Find, report and delete every stale marker of `app_id` in `dir`. Markers
/// of live sessions are locked and skipped. `logs` is searched for the
/// session's newest log.
pub(crate) fn take_stale(dir: &Path, app_id: &str, logs: Option<&Path>) -> Vec<UncleanExit> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let app = sanitize(app_id);
    let mut stale = Vec::new();
    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name
            .strip_suffix(EXTENSION)
            .and_then(|s| s.strip_suffix('.'))
        else {
            continue;
        };
        if !is_session_file(&format!("{stem}.nlog"), &app) {
            continue;
        }
        let Ok(mut file) = OpenOptions::new().read(true).write(true).open(&path) else {
            continue;
        };
        if file.try_lock().is_err() {
            // A live session, or another starting session took it first.
            continue;
        }
        let mut text = String::new();
        let _ = file.read_to_string(&mut text);
        let modified = file
            .metadata()
            .and_then(|m| m.modified())
            .map_or(0, unix_ns);
        let _ = fs::remove_file(&path);
        drop(file);
        stale.push(parse(stem, &text, modified, dir, logs));
    }
    stale.sort_by_key(|s| s.start_unix_ns);
    stale
}

fn parse(stem: &str, text: &str, modified: u64, crash: &Path, logs: Option<&Path>) -> UncleanExit {
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    };
    let number = |key: &str| {
        value(key).and_then(|v| match v.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16).ok(),
            None => v.parse().ok(),
        })
    };
    // The pid is also the stem's last part, for a marker that died empty.
    let pid = number("pid").or_else(|| stem.rsplit('-').next()?.parse().ok());
    let mut last = number("last_unix_ns").unwrap_or(0).max(modified);
    let mut fatal_snapshot = None;
    for dir in [Some(crash), logs].into_iter().flatten() {
        let Ok(read) = fs::read_dir(dir) else {
            continue;
        };
        for entry in read.filter_map(Result::ok) {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(rest) = name.strip_prefix(stem) else {
                continue;
            };
            if !name.ends_with(".nlog") || !(rest.starts_with('.') || rest.starts_with('-')) {
                continue;
            }
            if let Ok(time) = entry.metadata().and_then(|m| m.modified()) {
                last = last.max(unix_ns(time));
            }
            let fatal = crate::crash::snapshot_kind(name, stem)
                .is_some_and(|kind| kind != crate::crash::CrashKind::UncleanExit);
            if fatal && dir == crash {
                fatal_snapshot = Some(entry.path());
            }
        }
    }
    UncleanExit {
        stem: stem.to_owned(),
        session_id: number("session_id").unwrap_or(0),
        pid: pid.unwrap_or(0),
        start_unix_ns: number("start_unix_ns").unwrap_or(0),
        last_write_unix_ns: last,
        app_version: value("app_version").unwrap_or_default().to_owned(),
        exception: number("exception_code")
            .map(|code| (code, number("exception_address").unwrap_or(0))),
        crash_snapshot: fatal_snapshot,
    }
}

fn unix_ns(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, crate::record::saturating_ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nana-diag-marker-{name}-{}-{seq}",
            std::process::id(),
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn meta(pid: u32, start: u64) -> SessionMetadata {
        let mut meta = SessionMetadata::new("dev.nana.marker", "Marker", "1.2.3");
        meta.pid = pid;
        meta.wall_start_unix_ns = start;
        meta.session_id = 0xfeed_0000 + u64::from(pid);
        meta
    }

    fn stem(meta: &SessionMetadata) -> String {
        crate::files::session_stem(&meta.app_id, meta.wall_start_unix_ns, meta.pid)
    }

    #[test]
    fn a_marker_left_behind_is_reported_once_with_its_session() {
        let dir = temp_dir("stale");
        let start = 1_790_000_000_000_000_000;
        let old = meta(41, start);
        let mut marker = RunMarker::create(&dir, &stem(&old), &old).unwrap();
        marker.heartbeat(start + 5_000_000_000);
        marker.record_exception(0xc000_0005, 0x7ff6_1234, start + 6_000_000_000);
        // The process "dies": the handle closes, the file stays.
        drop(marker);

        let stale = take_stale(&dir, "dev.nana.marker", None);
        assert_eq!(stale.len(), 1, "{stale:?}");
        let s = &stale[0];
        assert_eq!(s.session_id, old.session_id);
        assert_eq!(s.pid, 41);
        assert_eq!(s.start_unix_ns, start);
        assert!(s.last_write_unix_ns >= start + 6_000_000_000);
        assert_eq!(s.app_version, "1.2.3");
        assert_eq!(s.exception, Some((0xc000_0005, 0x7ff6_1234)));
        assert_eq!(s.crash_snapshot, None);
        // Reported and gone.
        assert!(take_stale(&dir, "dev.nana.marker", None).is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn live_and_cleanly_removed_markers_are_not_stale() {
        let dir = temp_dir("live");
        let live = meta(42, 1_790_000_000_000_000_000);
        let held = RunMarker::create(&dir, &stem(&live), &live).unwrap();
        let clean = meta(43, 1_790_000_001_000_000_000);
        RunMarker::create(&dir, &stem(&clean), &clean)
            .unwrap()
            .remove();
        // Another app's leftover marker is not ours to report.
        let other = SessionMetadata::new("dev.nana.other", "Other", "1");
        drop(RunMarker::create(&dir, &stem(&other), &other).unwrap());

        assert!(take_stale(&dir, "dev.nana.marker", None).is_empty());
        held.remove();
        assert_eq!(take_stale(&dir, "dev.nana.other", None).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_fatal_snapshot_and_newer_logs_belong_to_the_stale_session() {
        let dir = temp_dir("files");
        let logs = dir.join("logs");
        fs::create_dir_all(&logs).unwrap();
        let old = meta(44, 1_000_000_000_000_000_000);
        let stem = stem(&old);
        drop(RunMarker::create(&dir, &stem, &old).unwrap());
        fs::write(dir.join(format!("{stem}-native-crash.nlog")), b"x").unwrap();
        fs::write(dir.join(format!("{stem}-stall.nlog")), b"x").unwrap();
        fs::write(logs.join(format!("{stem}.nlog")), b"x").unwrap();

        let stale = take_stale(&dir, "dev.nana.marker", Some(&logs));
        assert_eq!(stale.len(), 1);
        assert_eq!(
            stale[0].crash_snapshot,
            Some(dir.join(format!("{stem}-native-crash.nlog")))
        );
        // The log was written just now, long after the 2001 start.
        assert!(stale[0].last_write_unix_ns > crate::session::unix_now_ns() - 60_000_000_000);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fixed_text_truncates_instead_of_allocating() {
        use fmt::Write as _;
        let mut text = FixedText::<8>::new();
        let _ = write!(text, "0123{}", 456_789);
        assert_eq!(text.as_bytes(), b"01234567");
    }
}
