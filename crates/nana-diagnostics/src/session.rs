//! Session identity and the knobs of one diagnostics runtime.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::schema::Severity;

/// Who is logging. Written into every `.nlog` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadata {
    pub app_id: String,
    pub app_name: String,
    pub app_version: String,
    pub build_id: Option<String>,
    /// Version of the framework hosting the app (NanaUI's, by default this
    /// crate's own).
    pub framework_version: String,
    /// Free-form `key = value` pairs known at startup. Facts known later (GPU
    /// adapter, backend) go through `set_session_info` instead.
    pub extra: Vec<(String, String)>,
    pub session_id: u64,
    pub pid: u32,
    pub wall_start_unix_ns: u64,
}

impl SessionMetadata {
    pub fn new(
        app_id: impl Into<String>,
        app_name: impl Into<String>,
        app_version: impl Into<String>,
    ) -> Self {
        let wall_start_unix_ns = unix_now_ns();
        let pid = std::process::id();
        Self {
            app_id: app_id.into(),
            app_name: app_name.into(),
            app_version: app_version.into(),
            build_id: None,
            framework_version: env!("CARGO_PKG_VERSION").to_owned(),
            extra: Vec::new(),
            // Unique enough to tell sessions apart; not a security token.
            session_id: wall_start_unix_ns ^ (u64::from(pid) << 32).rotate_left(17),
            pid,
            wall_start_unix_ns,
        }
    }

    pub fn build_id(mut self, build_id: impl Into<String>) -> Self {
        self.build_id = Some(build_id.into());
        self
    }

    pub fn framework_version(mut self, version: impl Into<String>) -> Self {
        self.framework_version = version.into();
        self
    }

    pub fn extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.push((key.into(), value.into()));
        self
    }
}

pub(crate) fn unix_now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

/// Where files go. `None` keeps that output in memory only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticsPaths {
    /// Session logs (`.nlog`), rotated and pruned.
    pub logs: Option<PathBuf>,
    /// Flight-recorder snapshots written on panic, device loss, or request.
    pub crash: Option<PathBuf>,
}

impl DiagnosticsPaths {
    pub fn new(logs: impl Into<PathBuf>, crash: impl Into<PathBuf>) -> Self {
        Self {
            logs: Some(logs.into()),
            crash: Some(crash.into()),
        }
    }

    pub fn in_memory() -> Self {
        Self::default()
    }
}

/// What reaches the session log on disk. The in-memory flight recorder
/// always keeps everything at or above `min_severity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistMode {
    /// No session log. Snapshots are still written to the crash directory.
    Off,
    /// Warn+ events, faults, metric snapshots, session info, markers. Bounded
    /// disk traffic for long release runs.
    Essential,
    /// Every recorded event.
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retention {
    /// Rotate the session log past this size.
    pub max_file_bytes: u64,
    /// Prune old logs until the directory is under this total.
    pub max_total_bytes: u64,
    pub max_files: usize,
    pub max_age: Duration,
    pub max_crash_files: usize,
    /// Files modified more recently than this are never pruned: they may
    /// belong to another running instance.
    pub live_grace: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            max_file_bytes: 16 * 1024 * 1024,
            max_total_bytes: 128 * 1024 * 1024,
            max_files: 32,
            max_age: Duration::from_secs(14 * 24 * 60 * 60),
            max_crash_files: 16,
            live_grace: Duration::from_secs(10 * 60),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticsConfig {
    /// Off: nothing is recorded and every call site costs one relaxed load.
    pub enabled: bool,
    /// Events below this are rejected at the call site.
    pub min_severity: Severity,
    /// Aggregate and snapshot metrics.
    pub metrics: bool,
    /// Per-thread event ring (rounded up to a power of two).
    pub ring_capacity: usize,
    /// Per-thread emergency ring for faults.
    pub fault_ring_capacity: usize,
    /// How often the worker drains the rings.
    pub poll_interval: Duration,
    /// Flight recorder keeps this much recent history...
    pub flight_window: Duration,
    /// ...but never more than this many encoded bytes.
    pub flight_bytes: usize,
    /// Write the pending batch once it reaches this size...
    pub batch_bytes: usize,
    /// ...or once it is this old.
    pub batch_interval: Duration,
    pub metric_interval: Duration,
    /// Monotonic ↔ wall-clock pairs, so readers can place events in real
    /// time across system sleep.
    pub clock_sync_interval: Duration,
    pub persist: PersistMode,
    pub retention: Retention,
    /// Install a panic hook that writes a flight-recorder snapshot.
    pub panic_hook: bool,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_severity: if cfg!(debug_assertions) {
                Severity::Debug
            } else {
                Severity::Info
            },
            metrics: true,
            ring_capacity: 2048,
            fault_ring_capacity: 64,
            poll_interval: Duration::from_millis(50),
            flight_window: Duration::from_secs(60),
            flight_bytes: 4 * 1024 * 1024,
            batch_bytes: 64 * 1024,
            batch_interval: Duration::from_secs(2),
            metric_interval: Duration::from_secs(10),
            clock_sync_interval: Duration::from_secs(60),
            persist: if cfg!(debug_assertions) {
                PersistMode::All
            } else {
                PersistMode::Essential
            },
            retention: Retention::default(),
            panic_hook: true,
        }
    }
}

impl DiagnosticsConfig {
    /// Nothing recorded.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}
