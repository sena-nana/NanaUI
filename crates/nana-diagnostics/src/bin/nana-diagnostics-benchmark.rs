//! Hot-path cost of diagnostics call sites (Issue #227 acceptance).
//!
//! `cargo run --release -p nana-diagnostics --features benchmark --bin nana-diagnostics-benchmark [out.json]`
//!
//! Reports ns/op for event / metric / span call sites with diagnostics off,
//! on, with the ring full, and with the sink stalled, plus the per-call
//! latency distribution while the worker is writing to a stalled sink.
//! Prints JSON; also writes it to `target/performance/diagnostics-benchmark.json`
//! (or the path given as the first argument).

use std::hint::black_box;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nana_diagnostics::{
    DiagnosticsConfig, DiagnosticsPaths, Domain, EventDescriptor, FieldDescriptor, HistogramCells,
    Metric, PersistMode, SessionMetadata, Severity, Sink, event, metric, span,
};

const D: Domain = Domain(0x7E00);
static EVENT: EventDescriptor = EventDescriptor::new(
    D,
    1,
    "bench.event",
    Severity::Info,
    &[FieldDescriptor::u64("a"), FieldDescriptor::f64("b")],
);
static COUNTER: Metric = Metric::counter(D, 1, "bench.counter", "count");
static HIST_CELLS: HistogramCells = HistogramCells::new();
static HIST: Metric = Metric::histogram(D, 2, "bench.hist", "ns", &HIST_CELLS);
static SPAN_CELLS: HistogramCells = HistogramCells::new();
static SPAN: Metric = Metric::histogram(D, 3, "bench.span", "ns", &SPAN_CELLS);

const OPS: u64 = 2_000_000;
const ROUNDS: usize = 7;

/// Best-of-`ROUNDS` ns/op (the machine is shared; min is the stable read).
fn ns_per_op(mut f: impl FnMut(u64)) -> f64 {
    (0..ROUNDS)
        .map(|_| {
            let start = Instant::now();
            for i in 0..OPS {
                f(black_box(i));
            }
            start.elapsed().as_nanos() as f64 / OPS as f64
        })
        .fold(f64::INFINITY, f64::min)
}

fn suite() -> Vec<(&'static str, f64)> {
    vec![
        (
            "event",
            ns_per_op(|i| event!(EVENT, a = i, b = i as f64 * 0.5)),
        ),
        ("counter", ns_per_op(|i| metric!(COUNTER, i & 1))),
        ("histogram", ns_per_op(|i| metric!(HIST, i))),
        (
            "span",
            ns_per_op(|_| {
                let _span = span!(SPAN);
            }),
        ),
    ]
}

struct StalledSink(Arc<AtomicBool>);

impl Sink for StalledSink {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<()> {
        while self.0.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
    fn flush(&mut self, _durable: bool) -> io::Result<()> {
        Ok(())
    }
}

fn config(persist: PersistMode, ring: usize, poll: Duration) -> DiagnosticsConfig {
    DiagnosticsConfig {
        min_severity: Severity::Info,
        persist,
        ring_capacity: ring,
        poll_interval: poll,
        batch_bytes: 16 * 1024,
        panic_hook: false,
        ..DiagnosticsConfig::default()
    }
}

fn meta() -> SessionMetadata {
    SessionMetadata::new("dev.nana.bench", "Bench", env!("CARGO_PKG_VERSION"))
}

fn percentile(sorted: &[u64], q: f64) -> u64 {
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

fn json_suite(out: &mut String, name: &str, rows: &[(&str, f64)]) {
    out.push_str(&format!("  \"{name}\": {{"));
    for (i, (op, ns)) in rows.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("\"{op}_ns\": {ns:.2}"));
    }
    out.push_str("},\n");
}

fn main() {
    let out_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/performance/diagnostics-benchmark.json".into());

    // 1. Nothing installed.
    let disabled = suite();

    // 2. Installed, draining normally, memory only.
    let guard = nana_diagnostics::install(
        config(PersistMode::Off, 4096, Duration::from_millis(5)),
        meta(),
        DiagnosticsPaths::in_memory(),
    )
    .unwrap();
    nana_diagnostics::register_thread();
    let enabled = suite();
    let stats_enabled = guard.diagnostics().stats();
    drop(guard);

    // 3. Installed but the worker never drains: every event hits a full ring.
    let guard = nana_diagnostics::install(
        config(PersistMode::Off, 64, Duration::from_secs(3600)),
        meta(),
        DiagnosticsPaths::in_memory(),
    )
    .unwrap();
    nana_diagnostics::register_thread();
    let full = vec![("event", ns_per_op(|i| event!(EVENT, a = i, b = 0.0)))];
    let stats_full = guard.diagnostics().stats();
    drop(guard);

    // 4. Worker blocked inside a sink write: per-call latency distribution.
    let stalled_flag = Arc::new(AtomicBool::new(true));
    let guard = nana_diagnostics::install(
        config(PersistMode::All, 4096, Duration::from_millis(5)),
        meta(),
        DiagnosticsPaths::in_memory(),
    )
    .unwrap();
    guard
        .diagnostics()
        .set_sink(Box::new(StalledSink(stalled_flag.clone())));
    nana_diagnostics::register_thread();
    let mut samples = Vec::with_capacity(1_000_000);
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut i = 0u64;
    while Instant::now() < deadline {
        let t = Instant::now();
        event!(EVENT, a = i, b = 1.0);
        samples.push(t.elapsed().as_nanos() as u64);
        i += 1;
    }
    samples.sort_unstable();
    let stats_stalled = guard.diagnostics().stats();
    stalled_flag.store(false, Ordering::Relaxed);
    drop(guard);

    // 5. Four threads hammering one histogram (contended atomics).
    let guard = nana_diagnostics::install(
        config(PersistMode::Off, 4096, Duration::from_millis(5)),
        meta(),
        DiagnosticsPaths::in_memory(),
    )
    .unwrap();
    let threads = 4;
    let contended = (0..ROUNDS)
        .map(|_| {
            let start = Instant::now();
            std::thread::scope(|scope| {
                for _ in 0..threads {
                    scope.spawn(|| {
                        for i in 0..OPS / 4 {
                            metric!(HIST, black_box(i));
                        }
                    });
                }
            });
            start.elapsed().as_nanos() as f64 / (OPS / 4) as f64
        })
        .fold(f64::INFINITY, f64::min);
    drop(guard);

    let mut json = String::from("{\n  \"schema\": \"nana-diagnostics-benchmark/1\",\n");
    json.push_str(&format!(
        "  \"ops_per_round\": {OPS}, \"rounds\": {ROUNDS},\n"
    ));
    json_suite(&mut json, "disabled", &disabled);
    json_suite(&mut json, "enabled", &enabled);
    json_suite(&mut json, "ring_full", &full);
    json.push_str(&format!(
        "  \"stalled_sink_event_latency_ns\": {{\"samples\": {}, \"p50\": {}, \"p99\": {}, \"p999\": {}, \"max\": {}}},\n",
        samples.len(),
        percentile(&samples, 0.5),
        percentile(&samples, 0.99),
        percentile(&samples, 0.999),
        samples.last().copied().unwrap_or(0)
    ));
    json.push_str(&format!(
        "  \"contended_histogram_4_threads_ns_per_op_per_thread\": {contended:.2},\n"
    ));
    json.push_str(&format!(
        "  \"dropped\": {{\"enabled\": {}, \"ring_full\": {}, \"stalled_sink\": {}}}\n}}\n",
        stats_enabled.events_dropped, stats_full.events_dropped, stats_stalled.events_dropped
    ));
    print!("{json}");
    if let Some(parent) = std::path::Path::new(&out_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&out_path, &json) {
        eprintln!("could not write {out_path}: {e}");
    }
}
