//! A panic writes a flight-recorder snapshot before the process goes down.
//! The panicking half runs in a child process (this same test binary).

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime};

use nana_diagnostics::nlog::{self, Entry};
use nana_diagnostics::{
    DiagnosticsConfig, DiagnosticsPaths, Domain, EventDescriptor, FieldDescriptor, PersistMode,
    SessionMetadata, Severity, event,
};

const CHILD_ENV: &str = "NANA_DIAGNOSTICS_PANIC_CHILD";
static BEFORE: EventDescriptor = EventDescriptor::new(
    Domain(0x0400),
    1,
    "panic.before",
    Severity::Info,
    &[FieldDescriptor::u64("step")],
);

#[test]
fn panic_hook_writes_a_snapshot_with_the_panic_message() {
    if let Ok(dir) = std::env::var(CHILD_ENV) {
        let dir = PathBuf::from(dir);
        let _guard = nana_diagnostics::install(
            DiagnosticsConfig {
                persist: PersistMode::Essential,
                // The worker should not be what saves us (clamped to 60 s,
                // far longer than the child runs).
                poll_interval: Duration::from_secs(3600),
                ..DiagnosticsConfig::default()
            },
            SessionMetadata::new("dev.nana.panic", "Panic", "1"),
            DiagnosticsPaths::new(dir.join("logs"), dir.join("crash")),
        )
        .unwrap();
        for step in 0..3u64 {
            event!(BEFORE, step = step);
        }
        std::thread::Builder::new()
            .name("render".into())
            .spawn(|| panic!("simulated render failure"))
            .unwrap()
            .join()
            .ok();
        return;
    }

    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nana-diag-panic-{nanos}"));
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "panic_hook_writes_a_snapshot_with_the_panic_message",
            "--nocapture",
        ])
        .env(CHILD_ENV, &dir)
        .status()
        .unwrap();
    assert!(status.success());

    let snapshots: Vec<_> = std::fs::read_dir(dir.join("crash"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with("-panic.nlog"))
        .collect();
    assert_eq!(snapshots.len(), 1, "{snapshots:?}");
    let file = nlog::read_file(&snapshots[0]).unwrap();
    assert_eq!(file.header.reason, "snapshot:panic");
    let steps = file
        .entries
        .iter()
        .filter(|e| matches!(e, Entry::Event { key, .. } if file.event_name(*key) == Some("panic.before")))
        .count();
    assert_eq!(steps, 3);
    let message = file.entries.iter().find_map(|e| match e {
        Entry::Fault { key, message, .. } if file.event_name(*key) == Some("diagnostics.panic") => {
            message.clone()
        }
        _ => None,
    });
    let message = message.expect("panic fault");
    assert!(message.contains("thread 'render' panicked"), "{message}");
    assert!(message.contains("simulated render failure"), "{message}");
    let _ = std::fs::remove_dir_all(dir);
}
