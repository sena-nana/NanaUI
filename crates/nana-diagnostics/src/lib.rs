//! Low-overhead structured diagnostics for NanaUI applications (Issue #227).
//!
//! NanaUI owns the mechanism — collection, buffering, persistence, export;
//! applications own the semantics — which events and metrics exist and what
//! they mean. Logging is lossy by design; rendering is not.
//!
//! ```text
//! producer thread ──event!/fault!──▶ thread-local SPSC ring (drop on full)
//!                 ──metric!/span!──▶ static atomics
//!                                         │  polled, never signalled*
//!                                         ▼
//!                              nana-diagnostics worker (low priority)
//!                               ├─ flight recorder (last ~60 s, in memory)
//!                               └─ batch ──▶ one sequential write ──▶ .nlog
//! ```
//! (*faults wake the worker so they persist promptly.)
//!
//! Producer-path guarantees: no file I/O, no `fsync`, no waiting on the
//! worker, no back-pressure, no global lock, no formatting, no allocation
//! after a thread's first event (faults with a message allocate once). When
//! no instance is installed every call site costs one relaxed atomic load.
//!
//! ```
//! use nana_diagnostics::{
//!     Domain, EventDescriptor, FieldDescriptor, HistogramCells, Metric, Severity, event, metric,
//! };
//!
//! const LIVE: Domain = Domain(0x0100);
//! static MODEL_LOADED: EventDescriptor = EventDescriptor::new(
//!     LIVE, 1, "live.model_loaded", Severity::Info, &[FieldDescriptor::u64("bytes")],
//! );
//! static TRACK_CELLS: HistogramCells = HistogramCells::new();
//! static TRACK_NS: Metric = Metric::histogram(LIVE, 1, "live.track", "ns", &TRACK_CELLS);
//!
//! event!(MODEL_LOADED, bytes = 1024u64);
//! metric!(TRACK_NS, std::time::Duration::from_micros(250));
//! ```

mod clock;
mod crash;
mod export;
mod files;
pub mod framework;
mod metric;
pub mod nlog;
mod package;
mod priority;
mod record;
mod ring;
mod runtime;
mod schema;
mod session;
mod span;
mod worker;

pub use crash::install_panic_hook;
pub use export::{ExportOptions, histogram_quantile, to_json_lines, to_text};
pub use files::Sink;
pub use metric::{HISTOGRAM_BUCKETS, HistogramCells, HistogramSample, Metric, MetricValue};
pub use package::{PackageOptions, export_package_from};
pub use record::{Field, FieldValue};
pub use runtime::{
    __private, AlreadyInstalled, Diagnostics, DiagnosticsGuard, DiagnosticsStats, enabled, global,
    install, marker, metrics_enabled, register_thread, set_session_info, snapshot,
};
pub use schema::{
    Domain, EventDescriptor, FieldDescriptor, FieldKind, MAX_FIELDS, MetricDescriptor, MetricKind,
    SchemaKey, Severity,
};
pub use session::{DiagnosticsConfig, DiagnosticsPaths, PersistMode, Retention, SessionMetadata};
pub use span::SpanGuard;

/// Record a structured event on the global instance.
///
/// ```ignore
/// event!(SURFACE_LOST, window = id, attempt = 2u32);
/// ```
///
/// Field names are checked against the descriptor in debug builds only.
#[macro_export]
macro_rules! event {
    ($event:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        let event: &'static $crate::EventDescriptor = &$event;
        if $crate::enabled(event.severity) {
            $crate::__private::emit(event, &[$($crate::Field::new(stringify!($name), $value)),*]);
        }
    }};
}

/// Record a fault: an event that goes through the emergency ring and wakes
/// the worker. An optional message follows a `;` and is formatted only when
/// the fault is recorded.
///
/// ```ignore
/// fault!(DEVICE_LOST, reason = 1u64; "device lost: {message}");
/// ```
#[macro_export]
macro_rules! fault {
    ($event:expr $(, $name:ident = $value:expr)* $(,)? $(; $($message:tt)+)?) => {{
        let event: &'static $crate::EventDescriptor = &$event;
        if $crate::enabled(event.severity) {
            let message: ::std::option::Option<::std::boxed::Box<str>> = None;
            $(let message = Some(::std::format!($($message)+).into_boxed_str());)?
            $crate::__private::fault(
                event,
                &[$($crate::Field::new(stringify!($name), $value)),*],
                message,
            );
        }
    }};
}

/// Counter: add (default 1). Gauge: set. Histogram: record a sample.
/// Accepts integers, `bool`, and `Duration` (nanoseconds).
#[macro_export]
macro_rules! metric {
    ($metric:expr) => {
        $crate::metric!($metric, 1u64)
    };
    ($metric:expr, $value:expr) => {{
        if $crate::metrics_enabled() {
            $crate::Metric::record(&$metric, $crate::MetricValue::to_metric($value));
        }
    }};
}

/// Time the rest of the scope into a histogram: `let _span = span!(LAYOUT_NS);`
#[macro_export]
macro_rules! span {
    ($metric:expr) => {
        $crate::SpanGuard::new(&$metric)
    };
}
