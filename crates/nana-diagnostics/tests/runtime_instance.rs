//! Behaviour of a standalone `Diagnostics` instance: persistence, drop
//! policy, back-pressure isolation, rotation, and snapshots.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use nana_diagnostics::nlog::{self, Entry, MetricSample, NlogFile, Value};
use nana_diagnostics::{
    Diagnostics, DiagnosticsConfig, DiagnosticsPaths, Domain, EventDescriptor, Field,
    FieldDescriptor, HistogramCells, Metric, PersistMode, SessionMetadata, Severity, Sink,
};

const D: Domain = Domain(0x0200);
static INFO: EventDescriptor = EventDescriptor::new(
    D,
    1,
    "test.info",
    Severity::Info,
    &[FieldDescriptor::u64("n"), FieldDescriptor::bool("flag")],
);
static WARN: EventDescriptor = EventDescriptor::new(
    D,
    2,
    "test.warn",
    Severity::Warn,
    &[FieldDescriptor::f64("ratio")],
);
static FAULT: EventDescriptor = EventDescriptor::new(
    D,
    3,
    "test.fault",
    Severity::Error,
    &[FieldDescriptor::i64("code")],
);
static CONFLICT: EventDescriptor = EventDescriptor::new(D, 1, "test.other", Severity::Info, &[]);

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nana-diag-{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(persist: PersistMode) -> DiagnosticsConfig {
    DiagnosticsConfig {
        min_severity: Severity::Debug,
        persist,
        poll_interval: Duration::from_millis(5),
        panic_hook: false,
        // Metrics are process-wide statics and taking a histogram empties
        // it; only the metrics test may collect them.
        metrics: false,
        ..DiagnosticsConfig::default()
    }
}

fn start(name: &str, config: DiagnosticsConfig) -> (Diagnostics, PathBuf) {
    let dir = temp_dir(name);
    let diagnostics = Diagnostics::start(
        config,
        SessionMetadata::new("dev.nana.test", "Test", "1.2.3").build_id("abc"),
        DiagnosticsPaths::new(dir.join("logs"), dir.join("crash")),
    );
    (diagnostics, dir)
}

fn nlogs(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .map(|read| {
            read.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "nlog"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn events<'a>(file: &'a NlogFile, name: &str) -> Vec<&'a Entry> {
    file.entries
        .iter()
        .filter(|entry| match entry {
            Entry::Event { key, .. } | Entry::Fault { key, .. } => {
                file.event_name(*key) == Some(name)
            }
            _ => false,
        })
        .collect()
}

#[test]
fn session_log_round_trips_events_faults_threads_and_info() {
    let (diagnostics, dir) = start("roundtrip", config(PersistMode::All));
    diagnostics.set_session_info("gpu.adapter", "Test GPU");
    for n in 0..10u64 {
        diagnostics.emit(&INFO, &[Field::new("n", n), Field::new("flag", n % 2 == 0)]);
    }
    let worker = {
        let diagnostics = diagnostics.clone();
        std::thread::Builder::new()
            .name("render".into())
            .spawn(move || diagnostics.emit(&WARN, &[Field::new("ratio", 0.5f64)]))
            .unwrap()
    };
    worker.join().unwrap();
    diagnostics.fault(&FAULT, &[Field::new("code", -7i64)], Some("it broke"));
    diagnostics.shutdown();

    let logs = nlogs(&dir.join("logs"));
    assert_eq!(logs.len(), 1, "{logs:?}");
    let file = nlog::read_file(&logs[0]).unwrap();
    assert!(!file.truncated);
    assert_eq!(file.header.app_id, "dev.nana.test");
    assert_eq!(file.header.app_version, "1.2.3");
    assert_eq!(file.header.build_id, "abc");
    assert_eq!(file.header.reason, "session");
    assert_eq!(file.session_info("gpu.adapter"), Some("Test GPU"));

    let infos = events(&file, "test.info");
    assert_eq!(infos.len(), 10);
    let Entry::Event { values, .. } = infos[3] else {
        unreachable!()
    };
    assert_eq!(values, &[Value::U64(3), Value::Bool(false)]);

    let warns = events(&file, "test.warn");
    let Entry::Event { thread, values, .. } = warns[0] else {
        unreachable!()
    };
    assert_eq!(file.threads.get(thread).map(String::as_str), Some("render"));
    assert_eq!(values, &[Value::F64(0.5)]);

    let faults = events(&file, "test.fault");
    let Entry::Fault {
        values, message, ..
    } = faults[0]
    else {
        unreachable!()
    };
    assert_eq!(values, &[Value::I64(-7)]);
    assert_eq!(message.as_deref(), Some("it broke"));

    // Timestamps never go backwards inside the log.
    let ts: Vec<u64> = file
        .entries
        .iter()
        .filter(|e| matches!(e, Entry::Event { .. }))
        .map(Entry::ts_ns)
        .collect();
    assert!(ts.windows(2).all(|w| w[0] <= w[1]));
    assert!(matches!(file.entries.last(), Some(Entry::Marker { text, .. }) if text == "shutdown"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn essential_mode_persists_only_warnings_but_snapshots_keep_everything() {
    let (diagnostics, dir) = start("essential", config(PersistMode::Essential));
    diagnostics.emit(&INFO, &[Field::new("n", 1u64), Field::new("flag", true)]);
    diagnostics.emit(&WARN, &[Field::new("ratio", 2.0f64)]);
    let snapshot = diagnostics
        .snapshot_blocking("manual", Duration::from_secs(5))
        .unwrap();
    diagnostics.shutdown();

    let log = nlog::read_file(&nlogs(&dir.join("logs"))[0]).unwrap();
    assert!(events(&log, "test.info").is_empty());
    assert_eq!(events(&log, "test.warn").len(), 1);

    assert!(snapshot.starts_with(dir.join("crash")));
    let snap = nlog::read_file(&snapshot).unwrap();
    assert_eq!(snap.header.reason, "snapshot:manual");
    assert_eq!(events(&snap, "test.info").len(), 1);
    assert_eq!(events(&snap, "test.warn").len(), 1);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn full_ring_drops_newest_and_reports_it() {
    let (diagnostics, dir) = start(
        "drop",
        DiagnosticsConfig {
            ring_capacity: 8,
            // The worker must not drain while we fill the ring.
            poll_interval: Duration::from_secs(3600),
            ..config(PersistMode::All)
        },
    );
    for n in 0..100u64 {
        diagnostics.emit(&INFO, &[Field::new("n", n), Field::new("flag", false)]);
    }
    assert_eq!(diagnostics.stats().events_dropped, 92);
    diagnostics.shutdown();
    let log = nlog::read_file(&nlogs(&dir.join("logs"))[0]).unwrap();
    let kept: Vec<_> = events(&log, "test.info")
        .into_iter()
        .map(|e| match e {
            Entry::Event { values, .. } => values[0],
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(kept, (0..8).map(Value::U64).collect::<Vec<_>>());
    let dropped = log.entries.iter().find_map(|e| match e {
        Entry::Dropped { threads, .. } => Some(threads.clone()),
        _ => None,
    });
    assert_eq!(dropped.map(|t| t[0].1), Some(92));
    let _ = fs::remove_dir_all(dir);
}

struct SlowSink {
    writes: Arc<AtomicUsize>,
    delay: Duration,
}

impl Sink for SlowSink {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<()> {
        std::thread::sleep(self.delay);
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn flush(&mut self, _durable: bool) -> io::Result<()> {
        std::thread::sleep(self.delay);
        Ok(())
    }
}

#[test]
fn a_stalled_sink_never_blocks_producers() {
    let (diagnostics, dir) = start(
        "slow",
        DiagnosticsConfig {
            batch_bytes: 256,
            ..config(PersistMode::All)
        },
    );
    let writes = Arc::new(AtomicUsize::new(0));
    diagnostics.set_sink(Box::new(SlowSink {
        writes: writes.clone(),
        delay: Duration::from_millis(300),
    }));
    diagnostics.register_current_thread();
    let mut samples = Vec::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(900) {
        let t = Instant::now();
        diagnostics.emit(&INFO, &[Field::new("n", 1u64), Field::new("flag", true)]);
        samples.push(t.elapsed());
    }
    samples.sort_unstable();
    // The sink sleeps 300 ms per write: a producer that waited on it would
    // take at least that. Preemption on a loaded machine costs a few ms,
    // never 300.
    let worst = *samples.last().unwrap();
    let p999 = samples[samples.len() * 999 / 1000];
    assert!(
        worst < Duration::from_millis(250),
        "worst emit took {worst:?}"
    );
    assert!(p999 < Duration::from_millis(1), "p99.9 emit took {p999:?}");
    assert!(writes.load(Ordering::SeqCst) >= 1);
    diagnostics.shutdown();
    let _ = fs::remove_dir_all(dir);
}

struct FailingSink;

impl Sink for FailingSink {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<()> {
        Err(io::Error::other("disk full"))
    }
    fn flush(&mut self, _durable: bool) -> io::Result<()> {
        Err(io::Error::other("disk full"))
    }
}

#[test]
fn write_failures_are_counted_and_snapshots_still_work() {
    let (diagnostics, dir) = start("failing", config(PersistMode::All));
    diagnostics.set_sink(Box::new(FailingSink));
    diagnostics.emit(&WARN, &[Field::new("ratio", 1.0f64)]);
    assert!(diagnostics.flush(false, Duration::from_secs(5)));
    let stats = diagnostics.stats();
    assert!(stats.write_errors >= 1, "{stats:?}");
    assert!(stats.bytes_discarded > 0, "{stats:?}");
    let snapshot = diagnostics
        .snapshot_blocking("after-failure", Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        events(&nlog::read_file(snapshot).unwrap(), "test.warn").len(),
        1
    );
    diagnostics.shutdown();
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn truncated_files_decode_up_to_the_last_intact_chunk() {
    let (diagnostics, dir) = start("truncated", config(PersistMode::All));
    for n in 0..5u64 {
        diagnostics.emit(&INFO, &[Field::new("n", n), Field::new("flag", true)]);
        assert!(diagnostics.flush(false, Duration::from_secs(5)));
    }
    diagnostics.shutdown();
    let bytes = fs::read(&nlogs(&dir.join("logs"))[0]).unwrap();
    let whole = nlog::decode(&bytes).unwrap();
    assert!(!whole.truncated);
    let cut = nlog::decode(&bytes[..bytes.len() - 3]).unwrap();
    assert!(cut.truncated);
    assert_eq!(cut.entries.len(), whole.entries.len() - 1);
    let mut corrupt = bytes.clone();
    let middle = corrupt.len() / 2;
    corrupt[middle] ^= 0xFF;
    let corrupt = nlog::decode(&corrupt).unwrap();
    assert!(corrupt.truncated);
    assert!(corrupt.entries.len() < whole.entries.len());
    assert!(matches!(
        nlog::decode(b"not a log"),
        Err(nlog::DecodeError::NotNlog)
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rotation_starts_every_file_self_describing() {
    let mut config = config(PersistMode::All);
    config.retention.max_file_bytes = 1024;
    let (diagnostics, dir) = start("rotate", config);
    diagnostics.set_session_info("gpu.backend", "metal");
    for n in 0..400u64 {
        diagnostics.emit(&INFO, &[Field::new("n", n), Field::new("flag", true)]);
        if n % 20 == 0 {
            assert!(diagnostics.flush(false, Duration::from_secs(5)));
        }
    }
    diagnostics.shutdown();
    let logs = nlogs(&dir.join("logs"));
    assert!(logs.len() >= 3, "{logs:?}");
    let mut total = 0;
    for path in &logs {
        let file = nlog::read_file(path).unwrap();
        assert!(!file.truncated, "{path:?}");
        assert_eq!(file.session_info("gpu.backend"), Some("metal"), "{path:?}");
        for entry in &file.entries {
            if let Entry::Event { key, .. } = entry {
                assert!(file.event_schemas.contains_key(key), "{path:?}");
                total += 1;
            }
        }
    }
    assert_eq!(total, 400);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn retention_prunes_old_sessions_but_not_recent_ones() {
    let dir = temp_dir("retention");
    let logs = dir.join("logs");
    fs::create_dir_all(&logs).unwrap();
    let old = SystemTime::now() - Duration::from_secs(3600);
    for i in 0..5 {
        let path = logs.join(format!("dev.nana.test-2020010{i}T000000Z-1.nlog"));
        fs::write(&path, b"x").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old + Duration::from_secs(i))
            .unwrap();
    }
    // Another app's log is never touched.
    fs::write(logs.join("other.app-20200101T000000Z-1.nlog"), b"x").unwrap();
    let mut config = config(PersistMode::All);
    config.retention.max_files = 3;
    config.retention.live_grace = Duration::from_secs(60);
    let diagnostics = Diagnostics::start(
        config,
        SessionMetadata::new("dev.nana.test", "Test", "1"),
        DiagnosticsPaths::new(&logs, dir.join("crash")),
    );
    diagnostics.emit(&WARN, &[Field::new("ratio", 1.0f64)]);
    diagnostics.shutdown();
    let names: Vec<String> = nlogs(&logs)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    // Three files for this app remain: the two newest old ones plus ours.
    let ours: Vec<_> = names
        .iter()
        .filter(|n| n.starts_with("dev.nana.test-"))
        .collect();
    assert_eq!(ours.len(), 3, "{names:?}");
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("dev.nana.test-20200104"))
    );
    assert!(names.iter().any(|n| n.starts_with("other.app-")));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn exited_threads_are_drained_then_forgotten() {
    let (diagnostics, dir) = start("exit", config(PersistMode::All));
    let handles: Vec<_> = (0..4u64)
        .map(|n| {
            let diagnostics = diagnostics.clone();
            std::thread::spawn(move || {
                diagnostics.emit(&INFO, &[Field::new("n", n), Field::new("flag", true)]);
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert!(diagnostics.flush(false, Duration::from_secs(5)));
    assert_eq!(diagnostics.stats().producers, 0);
    diagnostics.shutdown();
    let log = nlog::read_file(&nlogs(&dir.join("logs"))[0]).unwrap();
    assert_eq!(events(&log, "test.info").len(), 4);
    let _ = fs::remove_dir_all(dir);
}

static HIST_CELLS: HistogramCells = HistogramCells::new();
static HIST: Metric = Metric::histogram(D, 1, "test.frame", "ns", &HIST_CELLS);
static COUNT: Metric = Metric::counter(D, 2, "test.count", "count");

#[test]
fn metric_snapshots_carry_counters_and_histograms() {
    let (diagnostics, dir) = start(
        "metrics",
        DiagnosticsConfig {
            metrics: true,
            ..config(PersistMode::Essential)
        },
    );
    for value in [100u64, 200, 400, 100_000] {
        HIST.record(value);
    }
    COUNT.record(3);
    diagnostics.shutdown();
    let log = nlog::read_file(&nlogs(&dir.join("logs"))[0]).unwrap();
    let samples: Vec<&MetricSample> = log
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Metrics { samples, .. } => Some(samples.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    let hist = samples
        .iter()
        .find_map(|s| match s {
            MetricSample::Histogram { key, sample }
                if log.metric_name(*key) == Some("test.frame") =>
            {
                Some(sample)
            }
            _ => None,
        })
        .expect("histogram snapshot");
    assert_eq!((hist.count, hist.min, hist.max), (4, 100, 100_000));
    assert!(samples.iter().any(|s| matches!(
        s,
        MetricSample::Counter { key, total: 3, .. } if log.metric_name(*key) == Some("test.count")
    )));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn conflicting_descriptors_are_counted_not_merged() {
    let (diagnostics, dir) = start("conflict", config(PersistMode::All));
    diagnostics.emit(&INFO, &[Field::new("n", 1u64), Field::new("flag", true)]);
    diagnostics.emit(&CONFLICT, &[]);
    assert!(diagnostics.flush(false, Duration::from_secs(5)));
    assert_eq!(diagnostics.stats().schema_conflicts, 1);
    diagnostics.shutdown();
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn disabled_instances_spawn_nothing_and_record_nothing() {
    let (diagnostics, dir) = start("disabled", DiagnosticsConfig::disabled());
    diagnostics.emit(&WARN, &[Field::new("ratio", 1.0f64)]);
    assert_eq!(diagnostics.stats().producers, 0);
    assert!(!diagnostics.flush(true, Duration::from_millis(10)));
    diagnostics.shutdown();
    assert!(nlogs(&dir.join("logs")).is_empty());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn package_export_collects_logs_snapshots_and_readable_exports() {
    let (diagnostics, dir) = start("package", config(PersistMode::All));
    diagnostics.fault(&FAULT, &[Field::new("code", 1i64)], Some("boom"));
    let package = diagnostics
        .export_package(
            &dir.join("out"),
            &nana_diagnostics::PackageOptions::default(),
        )
        .unwrap();
    diagnostics.shutdown();
    let manifest = fs::read_to_string(package.join("manifest.json")).unwrap();
    assert!(manifest.contains("\"kind\": \"session\""), "{manifest}");
    assert!(manifest.contains("\"kind\": \"snapshot\""), "{manifest}");
    let text: Vec<_> = fs::read_dir(package.join("session"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    let text = fs::read_to_string(&text[0]).unwrap();
    assert!(text.contains("FAULT test.fault"), "{text}");
    assert!(text.contains(":: boom"), "{text}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn requests_after_shutdown_return_immediately() {
    let (diagnostics, dir) = start("after-shutdown", config(PersistMode::All));
    diagnostics.shutdown();
    let started = Instant::now();
    assert!(!diagnostics.flush(true, Duration::from_secs(5)));
    diagnostics.marker("ignored");
    diagnostics.set_session_info("k", "v");
    // No worker: the snapshot is written from the calling thread.
    let snapshot = diagnostics
        .snapshot_blocking("late", Duration::from_secs(5))
        .unwrap();
    assert!(snapshot.exists());
    assert!(started.elapsed() < Duration::from_secs(2));
    let _ = fs::remove_dir_all(dir);
}

struct PanickingSink;

impl Sink for PanickingSink {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<()> {
        panic!("sink bug");
    }
    fn flush(&mut self, _durable: bool) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_panicking_sink_does_not_kill_the_worker() {
    let (diagnostics, dir) = start("panicking-sink", config(PersistMode::All));
    diagnostics.set_sink(Box::new(PanickingSink));
    diagnostics.emit(&WARN, &[Field::new("ratio", 1.0f64)]);
    assert!(diagnostics.flush(false, Duration::from_secs(5)));
    diagnostics.emit(&WARN, &[Field::new("ratio", 2.0f64)]);
    // Still alive: it answers, and the flight recorder kept both events.
    assert!(diagnostics.flush(false, Duration::from_secs(5)));
    let snapshot = diagnostics
        .snapshot_blocking("after-sink-panic", Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        events(&nlog::read_file(snapshot).unwrap(), "test.warn").len(),
        2
    );
    assert!(diagnostics.stats().write_errors >= 1);
    diagnostics.shutdown();
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_restart_in_the_same_second_keeps_the_earlier_log() {
    let dir = temp_dir("same-second");
    let meta = SessionMetadata::new("dev.nana.test", "Test", "1");
    let paths = DiagnosticsPaths::new(dir.join("logs"), dir.join("crash"));
    for ratio in [1.0f64, 2.0] {
        // Same metadata, so the same stem (app, second, pid).
        let diagnostics = Diagnostics::start(config(PersistMode::All), meta.clone(), paths.clone());
        diagnostics.emit(&WARN, &[Field::new("ratio", ratio)]);
        diagnostics.shutdown();
    }
    let logs = nlogs(&dir.join("logs"));
    assert_eq!(logs.len(), 2, "{logs:?}");
    for log in logs {
        assert_eq!(events(&nlog::read_file(log).unwrap(), "test.warn").len(), 1);
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn dropping_every_handle_stops_the_worker() {
    let (diagnostics, dir) = start("dropped", config(PersistMode::All));
    diagnostics.emit(&WARN, &[Field::new("ratio", 1.0f64)]);
    drop(diagnostics);
    // The worker notices within a poll interval and does a final flush.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let logs = nlogs(&dir.join("logs"));
        if let Some(log) = logs.first()
            && let Ok(file) = nlog::read_file(log)
            && matches!(file.entries.last(), Some(Entry::Marker { text, .. }) if text == "shutdown")
        {
            break;
        }
        assert!(Instant::now() < deadline, "worker never shut down");
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_flooding_session_keeps_its_own_logs_within_the_disk_budget() {
    let mut config = config(PersistMode::All);
    config.retention.max_file_bytes = 4 * 1024;
    config.retention.max_total_bytes = 16 * 1024;
    // The grace period protects *other* instances' recent files, not this
    // session's own rotations.
    config.retention.live_grace = Duration::from_secs(3600);
    let (diagnostics, dir) = start("flood", config);
    for n in 0..20_000u64 {
        diagnostics.emit(&INFO, &[Field::new("n", n), Field::new("flag", true)]);
        if n % 500 == 0 {
            assert!(diagnostics.flush(false, Duration::from_secs(5)));
        }
    }
    diagnostics.shutdown();
    let total: u64 = nlogs(&dir.join("logs"))
        .iter()
        .map(|p| fs::metadata(p).unwrap().len())
        .sum();
    // Budget plus the one file being written when the last prune ran.
    assert!(total <= 16 * 1024 + 8 * 1024, "logs grew to {total} bytes");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn essential_mode_writes_little_for_info_heavy_sessions() {
    let (diagnostics, dir) = start("essential-volume", config(PersistMode::Essential));
    for n in 0..50_000u64 {
        diagnostics.emit(&INFO, &[Field::new("n", n), Field::new("flag", true)]);
        if n % 1000 == 0 {
            assert!(diagnostics.flush(false, Duration::from_secs(5)));
        }
    }
    diagnostics.shutdown();
    let total: u64 = nlogs(&dir.join("logs"))
        .iter()
        .map(|p| fs::metadata(p).unwrap().len())
        .sum();
    // Info events stay in the flight recorder; the log holds the header,
    // schemas and markers only.
    assert!(total < 4 * 1024, "essential log is {total} bytes");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn repeated_snapshots_respect_the_crash_file_limit() {
    let mut config = config(PersistMode::Off);
    config.retention.max_crash_files = 3;
    let (diagnostics, dir) = start("snapshots", config);
    for n in 0..8u64 {
        diagnostics
            .snapshot_blocking(&format!("s{n}"), Duration::from_secs(5))
            .unwrap();
    }
    diagnostics.shutdown();
    assert_eq!(nlogs(&dir.join("crash")).len(), 3);
    let _ = fs::remove_dir_all(dir);
}
