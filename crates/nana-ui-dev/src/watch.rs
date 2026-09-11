//! Filesystem watch, debounce, and change classification.
//!
//! Split three ways on purpose: [`Debouncer`] and [`classify`] are pure and
//! unit-tested without touching the filesystem, and [`DevWatcher`] is the thin
//! layer that wires `notify` to them.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

use crate::ReloadRequest;
use crate::config::{DevConfig, is_ignored};

/// Collects raw filesystem events and releases them once the tree goes quiet.
///
/// A batch is released only when the *newest* recorded event is at least
/// `quiet` old. Releasing per-path instead would split one bundler emit across
/// several reloads.
#[derive(Debug)]
pub(crate) struct Debouncer {
    pending: HashMap<PathBuf, Instant>,
    quiet: Duration,
}

impl Debouncer {
    pub(crate) fn new(quiet: Duration) -> Self {
        Self {
            pending: HashMap::new(),
            quiet,
        }
    }

    /// Whether anything is waiting to settle.
    pub(crate) fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn record(&mut self, path: PathBuf, at: Instant) {
        self.pending
            .entry(path)
            .and_modify(|seen| {
                if at > *seen {
                    *seen = at;
                }
            })
            .or_insert(at);
    }

    /// Return the settled batch, or empty while the tree is still changing.
    ///
    /// Paths come back sorted so a multi-file emit produces a deterministic
    /// batch — the CSS fast path applies sheets in this order.
    pub(crate) fn take_ready(&mut self, now: Instant) -> Vec<PathBuf> {
        let Some(newest) = self.pending.values().copied().max() else {
            return Vec::new();
        };
        if now.saturating_duration_since(newest) < self.quiet {
            return Vec::new();
        }
        let mut paths: Vec<PathBuf> = self.pending.drain().map(|(path, _)| path).collect();
        paths.sort();
        paths
    }
}

/// Turn a settled batch into the smallest set of requests that covers it.
///
/// A path takes the CSS fast path only if it was registered with
/// [`DevConfig::css`]. Anything else — a script, an asset, an unregistered
/// stylesheet — means the artifact bytes may have changed, and the whole batch
/// collapses to a single [`ReloadRequest::Full`].
pub(crate) fn classify(paths: &[PathBuf], css: &BTreeSet<PathBuf>) -> Vec<ReloadRequest> {
    if paths.is_empty() {
        return Vec::new();
    }
    if paths.iter().any(|path| !css.contains(path)) {
        return vec![ReloadRequest::Full];
    }
    paths
        .iter()
        .map(|path| ReloadRequest::Css { path: path.clone() })
        .collect()
}

/// A file that cannot be read, or reads back empty, is a save caught between
/// truncate and write. Dropping it here is what keeps a reload from showing a
/// blank window and a syntax error that describes nothing the developer wrote.
fn is_settled_on_disk(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

/// Best-effort canonicalization: a path that does not exist yet keeps its
/// given form so a later create still matches the registered CSS set.
fn canonical_or_given(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Watches the configured roots and delivers settled, classified batches.
///
/// The watch lives exactly as long as this value: dropping it stops the OS
/// watch and joins the companion thread, so storing it in the program struct
/// ties it to the application's lifetime.
pub struct DevWatcher {
    watcher: Option<notify::RecommendedWatcher>,
    stop: Arc<AtomicBool>,
    wakeup: Arc<Condvar>,
    thread: Option<JoinHandle<()>>,
}

impl DevWatcher {
    /// Start watching. `on_batch` runs on the companion thread, so it must not
    /// block — the intended body is a `RuntimeProgramContext::dispatch` per
    /// request, which is a channel send plus an event-loop wake.
    pub fn spawn<F>(config: &DevConfig, on_batch: F) -> Result<Self, notify::Error>
    where
        F: Fn(Vec<ReloadRequest>) + Send + 'static,
    {
        let css: BTreeSet<PathBuf> = config
            .css_paths()
            .iter()
            .map(|path| canonical_or_given(path))
            .collect();
        let roots: Vec<PathBuf> = config
            .watch_roots()
            .iter()
            .map(|root| canonical_or_given(root))
            .collect();

        let quiet = config.quiet();
        let debouncer = Arc::new(Mutex::new(Debouncer::new(quiet)));
        let wakeup = Arc::new(Condvar::new());
        let sink = Arc::clone(&debouncer);
        let settled = Arc::clone(&wakeup);
        let filter_roots = roots.clone();

        // The notify callback runs on the platform's own notify thread. It
        // records and returns; every decision happens on the companion thread.
        let mut watcher =
            notify::recommended_watcher(move |event: Result<notify::Event, notify::Error>| {
                let Ok(event) = event else {
                    return;
                };
                if !event.kind.is_create() && !event.kind.is_modify() && !event.kind.is_remove() {
                    return;
                }
                // Canonicalization is filesystem I/O and belongs neither on
                // this thread nor under the lock: paths are recorded raw and
                // resolved once per settled batch, after the debouncer has
                // collapsed repeated writes to the same file.
                let interesting: Vec<PathBuf> = event
                    .paths
                    .into_iter()
                    .filter(|path| {
                        !filter_roots
                            .iter()
                            .any(|root| path.starts_with(root) && is_ignored(path, root))
                    })
                    .collect();
                if interesting.is_empty() {
                    return;
                }
                let now = Instant::now();
                if let Ok(mut pending) = sink.lock() {
                    for path in interesting {
                        pending.record(path, now);
                    }
                }
                // Wake the companion thread; it sleeps until there is something
                // to settle rather than polling.
                settled.notify_one();
            })?;

        for root in &roots {
            // notify only attaches the path on some backends: inotify reports a
            // bare `PathNotFound`, so a missing root would name nothing on
            // Linux while naming itself on macOS. The path is the whole content
            // of this error, so attach it here rather than depend on which
            // backend `recommended_watcher` picked.
            watcher
                .watch(root, RecursiveMode::Recursive)
                .map_err(|error| error.add_path(root.clone()))?;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_wakeup = Arc::clone(&wakeup);
        let thread = std::thread::Builder::new()
            .name("nana-dev-watch".into())
            .spawn(move || {
                let mut resolved: HashMap<PathBuf, PathBuf> = HashMap::new();
                while !thread_stop.load(Ordering::Relaxed) {
                    let ready = {
                        let Ok(pending) = debouncer.lock() else {
                            return;
                        };
                        // Sleep outright while nothing is pending; once
                        // something is, re-check no faster than the quiet
                        // period. An idle session costs no wakeups at all.
                        let mut pending = if pending.is_pending() {
                            thread_wakeup
                                .wait_timeout(pending, quiet)
                                .unwrap_or_else(|error| error.into_inner())
                                .0
                        } else {
                            thread_wakeup
                                .wait(pending)
                                .unwrap_or_else(|error| error.into_inner())
                        };
                        pending.take_ready(Instant::now())
                    };
                    let settled: Vec<PathBuf> = ready
                        .into_iter()
                        .filter(|path| is_settled_on_disk(path))
                        .map(|path| {
                            resolved
                                .entry(path.clone())
                                .or_insert_with(|| canonical_or_given(&path))
                                .clone()
                        })
                        .collect();
                    let batch = classify(&settled, &css);
                    if !batch.is_empty() {
                        on_batch(batch);
                    }
                }
            })
            .expect("spawn nana-dev-watch thread");

        Ok(Self {
            watcher: Some(watcher),
            stop,
            wakeup,
            thread: Some(thread),
        })
    }
}

impl Drop for DevWatcher {
    fn drop(&mut self) {
        // Stop the OS watch first so no further events land while we join.
        drop(self.watcher.take());
        self.stop.store(true, Ordering::Relaxed);
        // The companion thread is parked on the condvar, not on a timer.
        self.wakeup.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl std::fmt::Debug for DevWatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DevWatcher")
            .field("running", &!self.stop.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    #[test]
    fn a_batch_waits_for_the_newest_event_not_the_oldest() {
        let base = Instant::now();
        let mut debouncer = Debouncer::new(Duration::from_millis(120));
        debouncer.record(PathBuf::from("a.js"), base);
        debouncer.record(PathBuf::from("b.css"), at(base, 100));

        // `a.js` alone would have settled by now; `b.css` has not.
        assert!(debouncer.take_ready(at(base, 150)).is_empty());
        assert!(debouncer.is_pending());

        assert_eq!(
            debouncer.take_ready(at(base, 221)),
            [PathBuf::from("a.js"), PathBuf::from("b.css")]
        );
        assert!(!debouncer.is_pending());
    }

    #[test]
    fn an_idle_debouncer_releases_nothing() {
        let mut debouncer = Debouncer::new(Duration::from_millis(120));
        assert!(debouncer.take_ready(Instant::now()).is_empty());
    }

    #[test]
    fn only_registered_stylesheets_take_the_css_fast_path() {
        let css = BTreeSet::from([PathBuf::from("app.css"), PathBuf::from("theme.css")]);
        assert_eq!(
            classify(&[PathBuf::from("app.css")], &css),
            [ReloadRequest::Css {
                path: PathBuf::from("app.css")
            }]
        );
        assert_eq!(
            classify(
                &[PathBuf::from("app.css"), PathBuf::from("theme.css")],
                &css
            ),
            [
                ReloadRequest::Css {
                    path: PathBuf::from("app.css")
                },
                ReloadRequest::Css {
                    path: PathBuf::from("theme.css")
                }
            ]
        );
    }

    #[test]
    fn a_mixed_batch_collapses_to_one_full_reload() {
        let css = BTreeSet::from([PathBuf::from("app.css")]);
        assert_eq!(
            classify(
                &[PathBuf::from("app.css"), PathBuf::from("app.iife.js")],
                &css
            ),
            [ReloadRequest::Full]
        );
    }

    #[test]
    fn an_empty_batch_asks_for_nothing() {
        assert_eq!(classify(&[], &BTreeSet::new()), []);
    }

    #[test]
    fn a_half_written_file_is_not_settled() {
        let dir = crate::testing::scratch("watch-settled");

        let truncated = dir.join("app.js");
        std::fs::write(&truncated, b"").expect("write");
        assert!(!is_settled_on_disk(&truncated));

        std::fs::write(&truncated, b"globalThis.x = 1;").expect("write");
        assert!(is_settled_on_disk(&truncated));

        assert!(!is_settled_on_disk(&dir.join("missing.js")));
        assert!(!is_settled_on_disk(&dir), "a directory is not an artifact");
    }
}
