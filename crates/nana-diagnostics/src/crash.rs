//! Best-effort crash capture.
//!
//! Covered: Rust panics, including under `panic = "abort"` (the hook runs
//! before the abort), and explicit fatal paths that call
//! [`crate::Diagnostics::snapshot_blocking`].
//!
//! Not covered: native crashes (SIGSEGV, access violations, aborts from C
//! code). Doing file I/O from a signal handler is not async-signal-safe;
//! capturing those needs an out-of-process crash handler, which is outside
//! this crate.

use std::cell::Cell;
use std::panic::PanicHookInfo;
use std::sync::Once;
use std::time::Duration;

use crate::framework;
use crate::record::{FaultRecord, Record};
use crate::runtime::global;

/// How long a panicking thread waits for the worker's state lock.
const PANIC_WAIT: Duration = Duration::from_millis(500);

static HOOK: Once = Once::new();

thread_local! {
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

/// Chain a hook in front of the current panic hook: record a Fatal fault,
/// write a flight-recorder snapshot to the crash directory, then call the
/// previous hook. Installs once per process; a no-op while no global
/// instance is installed.
pub fn install_panic_hook() {
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            capture_panic(info);
            previous(info);
        }));
    });
}

fn capture_panic(info: &PanicHookInfo<'_>) {
    // A panic inside our own capture must not recurse.
    if IN_HOOK.with(|flag| flag.replace(true)) {
        return;
    }
    if let Some(diagnostics) = global() {
        let shared = &diagnostics.shared;
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string payload>");
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let message = format!("thread '{name}' panicked at {location}: {payload}");
        let fault = FaultRecord {
            record: Record {
                ts_ns: shared.now_ns(),
                event: &framework::diagnostics::PANIC,
                thread: 0,
                len: 0,
                values: [0; crate::schema::MAX_FIELDS],
            },
            message: Some(message.into_boxed_str()),
        };
        let _ = crate::worker::snapshot_now(shared, "panic", Some(fault), PANIC_WAIT);
    }
    IN_HOOK.with(|flag| flag.set(false));
}
