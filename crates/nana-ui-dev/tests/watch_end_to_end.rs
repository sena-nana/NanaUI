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
fn a_source_edit_arrives_as_one_full_rebuild() {
    let dir = scratch("e2e-script");
    let source = dir.join("main.rs");
    std::fs::write(&source, b"fn main() {}").expect("seed");

    let config = DevConfig::new_rust(&dir);
    let (_watcher, rx) = watch(&config);

    std::fs::write(&source, b"fn main() { run() }").expect("edit");

    assert_eq!(next_batch(&rx), [ReloadRequest::Full]);
    expect_quiet(&rx);
}

#[test]
fn a_truncated_save_is_dropped_and_the_completed_write_is_delivered() {
    let dir = scratch("e2e-truncated");
    let source = dir.join("main.rs");
    std::fs::write(&source, b"fn main() {}").expect("seed");

    let config = DevConfig::new_rust(&dir);
    let (_watcher, rx) = watch(&config);

    // Leave the file empty for longer than the quiet period: this is exactly the
    // window a slow editor opens between truncate and write, and rebuilding
    // here would compile an empty file.
    std::fs::write(&source, b"").expect("truncate");
    expect_quiet(&rx);

    std::fs::write(&source, b"fn main() { run() }").expect("finish the write");
    assert_eq!(next_batch(&rx), [ReloadRequest::Full]);
}

#[test]
fn writes_under_an_ignored_directory_never_reach_the_application() {
    let dir = scratch("e2e-ignored");
    let source = dir.join("main.rs");
    std::fs::write(&source, b"fn main() {}").expect("seed");
    let build_output = dir.join("target").join("debug");
    std::fs::create_dir_all(&build_output).expect("build_output dir");

    let config = DevConfig::new_rust(&dir);
    let (_watcher, rx) = watch(&config);

    // Reach a known-live, drained watcher before asserting silence. Two reasons,
    // both of which this test got wrong: FSEvents reports writes made just
    // before the stream opened, so the seed above could land inside the quiet
    // window and be read as a leaked build_output write; and without a batch that
    // must arrive, a watcher that never registered would satisfy `expect_quiet`
    // vacuously and the test would pass for the wrong reason.
    std::fs::write(&source, b"fn main() { run() }").expect("edit");
    assert_eq!(next_batch(&rx), [ReloadRequest::Full]);
    expect_quiet(&rx);

    std::fs::write(build_output.join("build-output"), b"artifact").expect("build output write");
    expect_quiet(&rx);
}

#[test]
fn dropping_the_watcher_stops_delivery() {
    let dir = scratch("e2e-drop");
    let source = dir.join("main.rs");
    std::fs::write(&source, b"fn main() {}").expect("seed");

    let config = DevConfig::new_rust(&dir);
    let (watcher, rx) = watch(&config);
    drop(watcher);

    std::fs::write(&source, b"fn main() { run() }").expect("edit");
    match rx.recv_timeout(Duration::from_millis(600)) {
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
        Ok(batch) => panic!("a dropped watcher still delivered {batch:?}"),
    }
}

#[test]
fn a_missing_watch_root_is_an_error_rather_than_a_silent_no_op() {
    let dir = scratch("e2e-missing-root");
    let config = DevConfig::new_rust(dir.join("nowhere"));
    let error = DevWatcher::spawn(&config, |_| {}).expect_err("watching a missing root fails");
    // The message names the path, which is what a developer needs to fix it.
    assert!(
        format!("{error}").contains("nowhere") || !error.paths.is_empty(),
        "unhelpful error: {error}"
    );
}

#[cfg(feature = "templates")]
#[test]
fn vue_style_edits_dispatch_in_process_without_running_rebuild() {
    use nana_ui_dev::{DevSignal, RebuildCommand, TemplateEvent, watch_template_events};

    let dir = scratch("e2e-vue-style-hot");
    let views_dir = dir.join("views");
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&views_dir).expect("views dir");
    std::fs::create_dir_all(&src_dir).expect("src dir");

    let vue_path = views_dir.join("Card.vue");
    let initial = r#"<script setup lang="rust">
let open = signal(false);
</script>
<template>
  <Column class="card" class:open="open">
    <Text>Hello</Text>
  </Column>
</template>
<style scoped>
.card { padding: 8px; opacity: 1; }
.card.open { opacity: 0.5; }
</style>"#;
    std::fs::write(&vue_path, initial).expect("seed vue");

    let (tx, rx) = channel();
    let config = DevConfig::new_rust(&src_dir);
    // Sentinel rebuild command: if invoked on a style edit, it returns Rebuilt.
    let rebuild = RebuildCommand::custom("true", Vec::new());
    let _watcher = watch_template_events(
        &config,
        &views_dir,
        "::nana_ui::runtime",
        rebuild,
        move |ev| {
            let _ = tx.send(ev);
        },
    )
    .expect("spawn template watcher");

    // 1. Static <style> edit dispatches TemplateEvent::Style without triggering rebuild.
    let style_edited = initial.replace("padding: 8px; opacity: 1;", "padding: 24px;");
    std::fs::write(&vue_path, &style_edited).expect("edit style");
    let event = rx
        .recv_timeout(DELIVERY_TIMEOUT)
        .expect("style hot event should arrive");
    match event {
        TemplateEvent::Style(style) => {
            assert_eq!(style.view, "Card");
            assert_eq!(style.styles.rules.len(), 2);
        }
        other => panic!("expected TemplateEvent::Style, got {other:?}"),
    }

    // 2. Script edit changes shape and triggers rebuild.
    let script_edited = style_edited.replace("signal(false)", "signal(true)");
    std::fs::write(&vue_path, &script_edited).expect("edit script");
    let event = rx
        .recv_timeout(DELIVERY_TIMEOUT)
        .expect("rebuild event should arrive");
    assert_eq!(event, TemplateEvent::Dev(DevSignal::Rebuilt));
}
