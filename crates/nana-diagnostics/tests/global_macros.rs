//! The global instance and the call-site macros. One test per process
//! section: the global is process-wide state.

use std::fs;
use std::time::{Duration, SystemTime};

use nana_diagnostics::nlog::{self, Entry, MetricSample};
use nana_diagnostics::{
    DiagnosticsConfig, DiagnosticsPaths, Domain, EventDescriptor, FieldDescriptor, HistogramCells,
    Metric, PersistMode, SessionMetadata, Severity, enabled, event, fault, metric, metrics_enabled,
    span,
};

const D: Domain = Domain(0x0300);
static HELLO: EventDescriptor = EventDescriptor::new(
    D,
    1,
    "global.hello",
    Severity::Info,
    &[FieldDescriptor::u64("n")],
);
static TRACE: EventDescriptor = EventDescriptor::new(D, 2, "global.trace", Severity::Trace, &[]);
static BROKEN: EventDescriptor = EventDescriptor::new(D, 3, "global.broken", Severity::Error, &[]);
static CELLS: HistogramCells = HistogramCells::new();
static WORK_NS: Metric = Metric::histogram(D, 1, "global.work", "ns", &CELLS);
static TICKS: Metric = Metric::counter(D, 2, "global.ticks", "count");

#[test]
fn macros_record_only_while_a_global_instance_is_installed() {
    // Nothing installed: every call site is a no-op.
    assert!(!enabled(Severity::Fatal));
    assert!(!metrics_enabled());
    event!(HELLO, n = 0u64);
    metric!(TICKS);
    assert_eq!(TICKS.value(), 0);

    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nana-diag-global-{nanos}"));
    let guard = nana_diagnostics::install(
        DiagnosticsConfig {
            min_severity: Severity::Debug,
            persist: PersistMode::All,
            poll_interval: Duration::from_millis(5),
            panic_hook: false,
            ..DiagnosticsConfig::default()
        },
        SessionMetadata::new("dev.nana.global", "Global", "0.1"),
        DiagnosticsPaths::new(dir.join("logs"), dir.join("crash")),
    )
    .unwrap();
    assert!(enabled(Severity::Info));
    assert!(!enabled(Severity::Trace));
    assert!(
        nana_diagnostics::install(
            DiagnosticsConfig::default(),
            SessionMetadata::new("x", "x", "x"),
            DiagnosticsPaths::in_memory(),
        )
        .is_err()
    );

    nana_diagnostics::register_thread();
    event!(HELLO, n = 7u64);
    event!(TRACE); // below min_severity
    let detail = "gpu";
    fault!(BROKEN; "lost {detail}");
    metric!(TICKS);
    metric!(TICKS, 4u32);
    {
        let _span = span!(WORK_NS);
        std::thread::sleep(Duration::from_millis(1));
    }
    nana_diagnostics::set_session_info("gpu.backend", "test");
    assert!(guard.diagnostics().flush(false, Duration::from_secs(5)));
    let log = guard.diagnostics().current_log();
    drop(guard);
    assert!(!enabled(Severity::Fatal));
    assert!(!metrics_enabled());

    let file = nlog::read_file(log.expect("session log was opened")).unwrap();
    let names: Vec<_> = file
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Event { key, .. } | Entry::Fault { key, .. } => file.event_name(*key),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["global.hello", "global.broken"]);
    assert!(file.entries.iter().any(|e| matches!(
        e,
        Entry::Fault { message: Some(m), .. } if m == "lost gpu"
    )));
    assert_eq!(file.session_info("gpu.backend"), Some("test"));
    let samples: Vec<_> = file
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Metrics { samples, .. } => Some(samples.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(samples.iter().any(|s| matches!(
        s,
        MetricSample::Counter { key, total: 5, .. } if file.metric_name(*key) == Some("global.ticks")
    )));
    assert!(samples.iter().any(|s| matches!(
        s,
        MetricSample::Histogram { key, sample } if file.metric_name(*key) == Some("global.work")
            && sample.count == 1 && sample.min >= 1_000_000
    )));

    // A fresh install works after the guard released the slot.
    let again = nana_diagnostics::install(
        DiagnosticsConfig {
            panic_hook: false,
            ..DiagnosticsConfig::default()
        },
        SessionMetadata::new("dev.nana.global", "Global", "0.2"),
        DiagnosticsPaths::in_memory(),
    )
    .unwrap();
    assert!(enabled(Severity::Error));
    drop(again);
    let _ = fs::remove_dir_all(dir);
}
