//! A panic writes a flight-recorder snapshot before the process goes down.
//! The panicking half runs in a child process (this same test binary).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

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

    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nana-diag-panic-{}-{seq}-{nanos}",
        std::process::id()
    ));
    let filter = match module_path!().split_once("::") {
        Some((_, prefix)) => {
            format!("{prefix}::panic_hook_writes_a_snapshot_with_the_panic_message")
        }
        None => "panic_hook_writes_a_snapshot_with_the_panic_message".to_owned(),
    };
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", &filter, "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, &dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || {
        use std::io::Read as _;
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_reader = std::thread::spawn(move || {
        use std::io::Read as _;
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = stdout_reader.join().unwrap_or_default();
                let stderr = stderr_reader.join().unwrap_or_default();
                panic!(
                    "panic_snapshot child `{filter}` timed out after 15s:\n{}\n{}",
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                );
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("failed waiting on panic_snapshot child `{filter}`: {err}");
            }
        }
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    assert!(
        status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );

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
