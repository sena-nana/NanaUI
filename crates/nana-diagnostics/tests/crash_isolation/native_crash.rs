//! A native crash writes a snapshot before the process goes down, and a
//! session that dies without any record is reported by the next one. The
//! dying half runs in a child process (this same test binary).
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, SystemTime};

use nana_diagnostics::nlog::{self, Entry, NlogFile, Value};
use nana_diagnostics::{
    CrashKind, Diagnostics, DiagnosticsConfig, DiagnosticsPaths, Domain, EventDescriptor,
    FieldDescriptor, PersistMode, SessionMetadata, Severity, event,
};

const CHILD_ENV: &str = "NANA_DIAGNOSTICS_NATIVE_CHILD";
const APP: &str = "dev.nana.native";
static BEFORE: EventDescriptor = EventDescriptor::new(
    Domain(0x0400),
    2,
    "native.before",
    Severity::Info,
    &[FieldDescriptor::u64("step")],
);

unsafe extern "system" {
    fn SetErrorMode(mode: u32) -> u32;
}

unsafe extern "C" {
    fn abort() -> !;
}

fn config() -> DiagnosticsConfig {
    DiagnosticsConfig {
        persist: PersistMode::All,
        // The worker should not be what saves us (clamped to 60 s, far
        // longer than the child runs).
        poll_interval: Duration::from_secs(3600),
        ..DiagnosticsConfig::default()
    }
}

fn paths(dir: &Path) -> DiagnosticsPaths {
    DiagnosticsPaths::new(dir.join("logs"), dir.join("crash"))
}

/// In the child: install diagnostics, log a little, then die by `crash`.
/// Returns false in the parent.
fn child(crash: impl FnOnce()) -> bool {
    let Ok(dir) = std::env::var(CHILD_ENV) else {
        return false;
    };
    // No "has stopped working" dialog to block the test.
    // SAFETY: plain Win32 call.
    unsafe { SetErrorMode(0x0001 | 0x0002) };
    let guard = nana_diagnostics::install(
        config(),
        SessionMetadata::new(APP, "Native", "1"),
        paths(Path::new(&dir)),
    )
    .unwrap();
    for step in 0..3u64 {
        event!(BEFORE, step = step);
    }
    crash();
    drop(guard);
    true
}

fn exact_filter(test: &str) -> String {
    match module_path!().split_once("::") {
        Some((_, prefix)) => format!("{prefix}::{test}"),
        None => test.to_owned(),
    }
}

fn run_child(test: &str) -> (PathBuf, ExitStatus) {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nana-diag-native-{test}-{}-{seq}-{nanos}",
        std::process::id()
    ));
    let filter = exact_filter(test);
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", &filter, "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, &dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
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
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = stdout_reader.join().unwrap_or_default();
                let stderr = stderr_reader.join().unwrap_or_default();
                panic!(
                    "native_crash child `{filter}` timed out after 15s:\n{}\n{}",
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                );
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("failed waiting on native_crash child `{filter}`: {err}");
            }
        }
    };
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    (dir, status)
}

fn crash_files(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    std::fs::read_dir(dir.join("crash"))
        .map(|read| {
            read.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.to_string_lossy().ends_with(suffix))
                .collect()
        })
        .unwrap_or_default()
}

fn faults<'a>(file: &'a NlogFile, name: &str) -> Vec<(&'a [Value], Option<&'a str>)> {
    file.entries
        .iter()
        .filter_map(|e| match e {
            Entry::Fault {
                key,
                values,
                message,
                ..
            } if file.event_name(*key) == Some(name) => {
                Some((values.as_slice(), message.as_deref()))
            }
            _ => None,
        })
        .collect()
}

fn the_native_crash(dir: &Path) -> (NlogFile, u64, String) {
    let snapshots = crash_files(dir, "-native-crash.nlog");
    assert_eq!(snapshots.len(), 1, "{snapshots:?}");
    let file = nlog::read_file(&snapshots[0]).unwrap();
    assert_eq!(file.header.reason, "snapshot:native-crash");
    let steps = file
        .entries
        .iter()
        .filter(|e| matches!(e, Entry::Event { key, .. } if file.event_name(*key) == Some("native.before")))
        .count();
    assert_eq!(steps, 3);
    let crashes = faults(&file, "diagnostics.native_crash");
    assert_eq!(crashes.len(), 1);
    let (values, message) = crashes[0];
    let Value::U64(code) = values[0] else {
        panic!("{values:?}")
    };
    let message = message.expect("message").to_owned();
    (file, code, message)
}

/// Start the next session on the same directories and return what it
/// found, then shut it down.
fn next_session(dir: &Path) -> (Vec<nana_diagnostics::UncleanExit>, Option<CrashKind>) {
    let diagnostics = Diagnostics::start(
        config(),
        SessionMetadata::new(APP, "Native", "2"),
        paths(dir),
    );
    let exits = diagnostics.unclean_exits().to_vec();
    let last = diagnostics.last_crash().map(|crash| crash.kind);
    diagnostics.shutdown();
    // Its own marker is gone again.
    assert!(
        crash_files(dir, ".running").is_empty(),
        "{:?}",
        crash_files(dir, ".running")
    );
    (exits, last)
}

/// The session log of the next session carries the unclean-exit fault.
fn logged_unclean_exit(dir: &Path, pid: u64) -> String {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir.join("logs")).unwrap() {
        let file = nlog::read_file(entry.unwrap().path()).unwrap();
        for (values, message) in faults(&file, "diagnostics.unclean_exit") {
            assert_eq!(values[1], Value::U64(pid));
            found.push(message.unwrap().to_owned());
        }
    }
    assert_eq!(found.len(), 1, "{found:?}");
    found.pop().unwrap()
}

#[inline(never)]
fn native_crash_test_access_violation() {
    // SAFETY: deliberately not safe: the write to 0x8 faults.
    unsafe { std::ptr::write_volatile(std::hint::black_box(std::ptr::dangling_mut::<u64>()), 1) };
}

#[test]
fn an_access_violation_writes_a_native_crash_snapshot() {
    if child(native_crash_test_access_violation) {
        return;
    }
    let (dir, status) = run_child("an_access_violation_writes_a_native_crash_snapshot");
    assert_eq!(status.code(), Some(0xC000_0005_u32 as i32), "{status:?}");

    let (_, code, message) = the_native_crash(&dir);
    assert_eq!(code, 0xC000_0005);
    assert!(message.contains("access violation"), "{message}");
    assert!(message.contains("writing 0x8"), "{message}");
    assert!(
        message.contains("native_crash_test_access_violation"),
        "{message}"
    );
    // The child's breadcrumb kept its marker; the next session reports it
    // and points at the snapshot instead of writing its own.
    let (exits, last) = next_session(&dir);
    assert_eq!(exits.len(), 1, "{exits:?}");
    assert_eq!(exits[0].exception.map(|e| e.0), Some(0xC000_0005));
    assert!(exits[0].crash_snapshot.is_some());
    assert_eq!(last, Some(CrashKind::NativeCrash));
    assert!(crash_files(&dir, "-unclean-exit.nlog").is_empty());
    let logged = logged_unclean_exit(&dir, exits[0].pid);
    assert!(logged.contains("native exception 0xc0000005"), "{logged}");
    assert!(logged.contains("-native-crash.nlog"), "{logged}");
    let _ = std::fs::remove_dir_all(dir);
}

#[inline(never)]
#[allow(unconditional_recursion)]
fn native_crash_test_recurse(depth: u64) -> u64 {
    let frame = std::hint::black_box([depth as u8; 1024]);
    native_crash_test_recurse(depth + 1) + u64::from(frame[0])
}

#[test]
fn a_stack_overflow_writes_a_native_crash_snapshot() {
    if child(|| {
        std::hint::black_box(native_crash_test_recurse(0));
    }) {
        return;
    }
    let (dir, status) = run_child("a_stack_overflow_writes_a_native_crash_snapshot");
    assert_eq!(status.code(), Some(0xC000_00FD_u32 as i32), "{status:?}");

    let (_, code, message) = the_native_crash(&dir);
    assert_eq!(code, 0xC000_00FD);
    assert!(message.contains("stack overflow"), "{message}");
    assert!(message.contains("native_crash_test_recurse"), "{message}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn c_abort_writes_a_native_crash_snapshot() {
    // SAFETY: deliberately ends the process.
    if child(|| unsafe { abort() }) {
        return;
    }
    let (dir, status) = run_child("c_abort_writes_a_native_crash_snapshot");
    assert!(!status.success(), "{status:?}");

    let (_, code, message) = the_native_crash(&dir);
    assert_eq!(code, 0x4000_0015);
    assert!(message.contains("abort() called"), "{message}");
    let (_, last) = next_session(&dir);
    assert_eq!(last, Some(CrashKind::NativeCrash));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_fail_fast_exit_is_reported_by_the_next_session() {
    // `std::process::abort` is a fail-fast: no handler in the process sees
    // it, exactly like being killed.
    if child(|| std::process::abort()) {
        return;
    }
    let (dir, status) = run_child("a_fail_fast_exit_is_reported_by_the_next_session");
    assert!(!status.success(), "{status:?}");
    assert!(crash_files(&dir, "-native-crash.nlog").is_empty());
    assert_eq!(crash_files(&dir, ".running").len(), 1);

    let (exits, last) = next_session(&dir);
    assert_eq!(exits.len(), 1, "{exits:?}");
    let exit = &exits[0];
    assert_eq!(exit.exception, None);
    assert_eq!(exit.crash_snapshot, None);
    assert_eq!(exit.app_version, "1");
    assert!(exit.start_unix_ns > 0 && exit.last_write_unix_ns >= exit.start_unix_ns);
    assert_eq!(last, Some(CrashKind::UncleanExit));
    let snapshots = crash_files(&dir, "-unclean-exit.nlog");
    assert_eq!(snapshots.len(), 1, "{snapshots:?}");
    let file = nlog::read_file(&snapshots[0]).unwrap();
    assert_eq!(faults(&file, "diagnostics.unclean_exit").len(), 1);
    let logged = logged_unclean_exit(&dir, exit.pid);
    assert!(logged.contains(&format!("pid {}", exit.pid)), "{logged}");
    assert!(logged.contains("no crash snapshot"), "{logged}");
    // Reported once: a third session finds nothing.
    let (exits, _) = next_session(&dir);
    assert!(exits.is_empty(), "{exits:?}");
    let _ = std::fs::remove_dir_all(dir);
}
