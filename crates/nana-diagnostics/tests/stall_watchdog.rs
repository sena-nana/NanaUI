//! A callback that does not return is reported from the watchdog thread,
//! and its end is recorded once it does.

use std::time::{Duration, Instant, SystemTime};

use nana_diagnostics::nlog::{self, Entry};
use nana_diagnostics::stall::{self, Callback, STALL_THRESHOLD};
use nana_diagnostics::{DiagnosticsConfig, DiagnosticsPaths, PersistMode, SessionMetadata};

/// Spins past the threshold, as a loop on the event-loop thread would.
#[inline(never)]
fn stall_watchdog_stuck_callback(until: Instant) {
    while Instant::now() < until {
        std::hint::spin_loop();
    }
}

#[test]
fn a_stuck_callback_is_reported_with_its_stack_and_its_recovery() {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nana-diag-stall-{nanos}"));
    let guard = nana_diagnostics::install(
        DiagnosticsConfig {
            persist: PersistMode::Essential,
            ..DiagnosticsConfig::default()
        },
        SessionMetadata::new("dev.nana.stall", "Stall", "1"),
        DiagnosticsPaths::new(dir.join("logs"), dir.join("crash")),
    )
    .unwrap();

    stall::watch_current_thread();
    {
        let _busy = stall::busy(Callback::WindowEvent);
        stall_watchdog_stuck_callback(Instant::now() + STALL_THRESHOLD + Duration::from_secs(1));
    }
    // One watchdog poll notices the return.
    std::thread::sleep(Duration::from_millis(600));
    drop(guard);

    let logs: Vec<_> = std::fs::read_dir(dir.join("logs"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "nlog"))
        .collect();
    assert_eq!(logs.len(), 1, "{logs:?}");
    let file = nlog::read_file(&logs[0]).unwrap();
    let message = file
        .entries
        .iter()
        .find_map(|entry| match entry {
            Entry::Fault { key, message, .. }
                if file.event_name(*key) == Some("diagnostics.main_thread_stalled") =>
            {
                message.clone()
            }
            _ => None,
        })
        .expect("stall fault");
    assert!(message.contains("inside window_event"), "{message}");
    if cfg!(all(windows, target_arch = "x86_64")) {
        assert!(message.contains("% of one core"), "{message}");
        assert!(message.contains("sample 1:"), "{message}");
        assert!(
            message.contains("stall_watchdog_stuck_callback"),
            "{message}"
        );
    } else {
        // Elsewhere the thread's CPU time and stack are not read.
        assert!(message.contains("no stack sample"), "{message}");
    }
    let recovered = file.entries.iter().any(|entry| {
        matches!(entry, Entry::Event { key, .. }
            if file.event_name(*key) == Some("diagnostics.main_thread_recovered"))
    });
    assert!(recovered, "no recovery event");
    let snapshots = std::fs::read_dir(dir.join("crash"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().to_string_lossy().ends_with("-stall.nlog"))
        .count();
    assert_eq!(snapshots, 1);
    let _ = std::fs::remove_dir_all(dir);
}
