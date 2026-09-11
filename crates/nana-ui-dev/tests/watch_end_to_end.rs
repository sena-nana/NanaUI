//! End-to-end cover for the `notify` wiring.
//!
//! The debounce and the classifier are unit-tested in-crate with a fake clock.
//! What those cannot reach is whether the watcher is actually registered, whether
//! events survive the platform backend, and whether the batch a real editor save
//! produces arrives as one request instead of several.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use nana_ui_dev::{DevConfig, DevWatcher, ReloadRequest};

/// Generous: FSEvents coalesces on its own schedule and CI machines are slow.
/// A batch that has not arrived in this long is a real failure, not a slow box.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::canonicalize(&dir).expect("canonicalize scratch dir")
}

fn watch(config: &DevConfig) -> (DevWatcher, Receiver<Vec<ReloadRequest>>) {
    let (tx, rx) = channel();
    let watcher = DevWatcher::spawn(config, move |batch| {
        let _ = tx.send(batch);
    })
    .expect("spawn watcher");
    (watcher, rx)
}

fn next_batch(rx: &Receiver<Vec<ReloadRequest>>) -> Vec<ReloadRequest> {
    rx.recv_timeout(DELIVERY_TIMEOUT)
        .expect("a batch should have been delivered")
}

fn expect_quiet(rx: &Receiver<Vec<ReloadRequest>>) {
    match rx.recv_timeout(Duration::from_millis(600)) {
        Err(RecvTimeoutError::Timeout) => {}
        Err(RecvTimeoutError::Disconnected) => panic!("watcher thread stopped early"),
        Ok(batch) => panic!("expected no batch, got {batch:?}"),
    }
}

#[test]
fn a_script_edit_arrives_as_one_full_reload() {
    let dir = scratch("e2e-script");
    let js = dir.join("app.iife.js");
    std::fs::write(&js, b"globalThis.x = 1;").expect("seed");

    let config = DevConfig::new(&js);
    let (_watcher, rx) = watch(&config);

    std::fs::write(&js, b"globalThis.x = 2;").expect("edit");

    assert_eq!(next_batch(&rx), [ReloadRequest::Full]);
    expect_quiet(&rx);
}

#[test]
fn a_registered_stylesheet_alone_takes_the_css_fast_path() {
    let dir = scratch("e2e-css");
    let js = dir.join("app.iife.js");
    let css = dir.join("app.css");
    std::fs::write(&js, b"globalThis.x = 1;").expect("seed js");
    std::fs::write(&css, b"body { color: red; }").expect("seed css");

    let config = DevConfig::new(&js).css(&css);
    let (_watcher, rx) = watch(&config);

    std::fs::write(&css, b"body { color: blue; }").expect("edit css");

    let canonical = std::fs::canonicalize(&css).expect("canonical css");
    assert_eq!(next_batch(&rx), [ReloadRequest::Css { path: canonical }]);
    expect_quiet(&rx);
}

#[test]
fn a_truncated_save_is_dropped_and_the_completed_write_is_delivered() {
    let dir = scratch("e2e-truncated");
    let js = dir.join("app.iife.js");
    std::fs::write(&js, b"globalThis.x = 1;").expect("seed");

    let config = DevConfig::new(&js);
    let (_watcher, rx) = watch(&config);

    // Leave the file empty for longer than the quiet period: this is exactly the
    // window a slow editor opens between truncate and write, and reloading here
    // would hand the engine zero bytes.
    std::fs::write(&js, b"").expect("truncate");
    expect_quiet(&rx);

    std::fs::write(&js, b"globalThis.x = 2;").expect("finish the write");
    assert_eq!(next_batch(&rx), [ReloadRequest::Full]);
}

#[test]
fn writes_under_an_ignored_directory_never_reach_the_application() {
    let dir = scratch("e2e-ignored");
    let js = dir.join("app.iife.js");
    std::fs::write(&js, b"globalThis.x = 1;").expect("seed");
    let vendored = dir.join("node_modules").join("vue");
    std::fs::create_dir_all(&vendored).expect("vendored dir");

    let config = DevConfig::new(&js);
    let (_watcher, rx) = watch(&config);

    // Reach a known-live, drained watcher before asserting silence. Two reasons,
    // both of which this test got wrong: FSEvents reports writes made just
    // before the stream opened, so the seed above could land inside the quiet
    // window and be read as a leaked vendored write; and without a batch that
    // must arrive, a watcher that never registered would satisfy `expect_quiet`
    // vacuously and the test would pass for the wrong reason.
    std::fs::write(&js, b"globalThis.x = 2;").expect("edit");
    assert_eq!(next_batch(&rx), [ReloadRequest::Full]);
    expect_quiet(&rx);

    std::fs::write(vendored.join("index.js"), b"module.exports = {};").expect("vendored write");
    expect_quiet(&rx);
}

#[test]
fn dropping_the_watcher_stops_delivery() {
    let dir = scratch("e2e-drop");
    let js = dir.join("app.iife.js");
    std::fs::write(&js, b"globalThis.x = 1;").expect("seed");

    let config = DevConfig::new(&js);
    let (watcher, rx) = watch(&config);
    drop(watcher);

    std::fs::write(&js, b"globalThis.x = 2;").expect("edit");
    match rx.recv_timeout(Duration::from_millis(600)) {
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
        Ok(batch) => panic!("a dropped watcher still delivered {batch:?}"),
    }
}

#[test]
fn a_missing_watch_root_is_an_error_rather_than_a_silent_no_op() {
    let dir = scratch("e2e-missing-root");
    let config = DevConfig::new(dir.join("nowhere").join("app.iife.js"));
    let error = DevWatcher::spawn(&config, |_| {}).expect_err("watching a missing root fails");
    // The message names the path, which is what a developer needs to fix it.
    assert!(
        format!("{error}").contains("nowhere") || !error.paths.is_empty(),
        "unhelpful error: {error}"
    );
}
