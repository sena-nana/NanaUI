//! The diagnostics runtime: producer registration, the emit path, the
//! process-wide default instance, and the control API.

use std::cell::{Cell, RefCell};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::record::{FaultRecord, Field, Record, pack};
use crate::ring::Ring;
use crate::schema::{EventDescriptor, Severity};
use crate::session::{DiagnosticsConfig, DiagnosticsPaths, SessionMetadata};
use crate::worker::{self, Request, WorkerState};

/// Lowest severity the global instance accepts; `u8::MAX` when none is
/// installed. The only thing a call site reads when diagnostics are off.
static THRESHOLD: AtomicU8 = AtomicU8::new(u8::MAX);
static METRICS_ON: AtomicBool = AtomicBool::new(false);
/// The installed instance. Installing leaks one strong reference, so a
/// producer that loaded the pointer can never observe it dangling.
static GLOBAL: AtomicPtr<Shared> = AtomicPtr::new(std::ptr::null_mut());
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// Would an event of `severity` be recorded by the global instance?
#[inline(always)]
pub fn enabled(severity: Severity) -> bool {
    severity as u8 >= THRESHOLD.load(Ordering::Relaxed)
}

/// Are metric call sites live?
#[inline(always)]
pub fn metrics_enabled() -> bool {
    METRICS_ON.load(Ordering::Relaxed)
}

pub(crate) struct Producer {
    pub(crate) thread: u32,
    pub(crate) name: String,
    pub(crate) events: Ring<Record>,
    pub(crate) faults: Ring<FaultRecord>,
    /// The owning thread exited; drain, then forget.
    pub(crate) retired: AtomicBool,
    /// The instance shut down; the thread-local entry can be dropped.
    orphaned: AtomicBool,
}

#[derive(Default)]
pub(crate) struct Stats {
    pub(crate) lost_no_producer: AtomicU64,
    pub(crate) write_errors: AtomicU64,
    pub(crate) bytes_written: AtomicU64,
    pub(crate) bytes_discarded: AtomicU64,
    pub(crate) schema_conflicts: AtomicU64,
    pub(crate) snapshots_written: AtomicU64,
    pub(crate) retired_events_dropped: AtomicU64,
    pub(crate) retired_faults_dropped: AtomicU64,
}

pub(crate) struct Shared {
    id: u64,
    pub(crate) config: DiagnosticsConfig,
    pub(crate) meta: SessionMetadata,
    pub(crate) paths: DiagnosticsPaths,
    start: Instant,
    min_severity: u8,
    pub(crate) producers: Mutex<Vec<Arc<Producer>>>,
    next_thread: AtomicU32,
    /// Consumer side of every ring plus all encoder state. Whoever holds it
    /// is the one consumer the SPSC rings allow.
    pub(crate) state: Mutex<WorkerState>,
    pub(crate) requests: Mutex<Vec<Request>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    /// Set once when the worker starts; unparking a finished thread is
    /// harmless, so it is never cleared. Lock-free for the fault path.
    worker_thread: OnceLock<std::thread::Thread>,
    /// False once the worker thread has exited, normally or by panic. Only
    /// changes under the `requests` lock (see `Shared::request`).
    worker_alive: AtomicBool,
    /// Set (and signalled) when the worker has finished its final flush, so
    /// every concurrent `shutdown` waits for it with a bound.
    worker_done: (Mutex<bool>, Condvar),
    pub(crate) shutdown: AtomicBool,
    pub(crate) stats: Stats,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct LocalProducers(RefCell<Vec<(u64, Arc<Producer>)>>);

impl Drop for LocalProducers {
    fn drop(&mut self) {
        for (_, producer) in self.0.get_mut().drain(..) {
            producer.retired.store(true, Ordering::Release);
        }
    }
}

thread_local! {
    static LOCAL: LocalProducers = const { LocalProducers(RefCell::new(Vec::new())) };
    pub(crate) static IS_WORKER: Cell<bool> = const { Cell::new(false) };
}

impl Shared {
    #[inline]
    pub(crate) fn now_ns(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    /// Run `f` with this thread's producer, registering one on first use.
    #[inline]
    fn with_producer(&self, f: impl FnOnce(&Producer)) {
        let done = LOCAL.try_with(|local| {
            if let Ok(list) = local.0.try_borrow() {
                if let Some((_, producer)) = list.iter().find(|(id, _)| *id == self.id) {
                    f(producer);
                    return true;
                }
            } else {
                return false;
            }
            match self.register(local) {
                Some(producer) => {
                    f(&producer);
                    true
                }
                None => false,
            }
        });
        if done != Ok(true) {
            self.stats.lost_no_producer.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cold]
    fn register(&self, local: &LocalProducers) -> Option<Arc<Producer>> {
        let mut list = local.0.try_borrow_mut().ok()?;
        list.retain(|(_, p)| !p.orphaned.load(Ordering::Acquire));
        if let Some((_, producer)) = list.iter().find(|(id, _)| *id == self.id) {
            return Some(producer.clone());
        }
        if self.shutdown.load(Ordering::Acquire) {
            return None;
        }
        let thread = self.next_thread.fetch_add(1, Ordering::Relaxed);
        let current = std::thread::current();
        let name = current
            .name()
            .map_or_else(|| format!("thread-{thread}"), str::to_owned);
        let producer = Arc::new(Producer {
            thread,
            name,
            events: Ring::new(self.config.ring_capacity),
            faults: Ring::new(self.config.fault_ring_capacity),
            retired: AtomicBool::new(false),
            orphaned: AtomicBool::new(false),
        });
        {
            let mut producers = lock(&self.producers);
            // Shutdown may have emptied the list since the check above; a
            // producer added now would never be drained or orphaned.
            if self.shutdown.load(Ordering::Acquire) {
                return None;
            }
            producers.push(producer.clone());
        }
        list.push((self.id, producer.clone()));
        Some(producer)
    }

    #[inline]
    pub(crate) fn emit(&self, event: &'static EventDescriptor, fields: &[Field]) {
        if (event.severity as u8) < self.min_severity {
            return;
        }
        let ts_ns = self.now_ns();
        let (len, values) = pack(event, fields);
        self.with_producer(|producer| {
            let record = Record {
                ts_ns,
                event,
                thread: producer.thread,
                len,
                values,
            };
            // SAFETY: a producer is reachable only from its own thread's LOCAL.
            unsafe { producer.events.push(record) };
        });
    }

    pub(crate) fn fault(
        &self,
        event: &'static EventDescriptor,
        fields: &[Field],
        message: Option<Box<str>>,
    ) {
        let ts_ns = self.now_ns();
        let (len, values) = pack(event, fields);
        let mut pushed = false;
        self.with_producer(|producer| {
            let fault = FaultRecord {
                record: Record {
                    ts_ns,
                    event,
                    thread: producer.thread,
                    len,
                    values,
                },
                message,
            };
            // SAFETY: as in `emit`.
            pushed = unsafe { producer.faults.push(fault) };
        });
        if pushed {
            // Faults are rare and worth persisting promptly. `unpark` never
            // blocks on the worker.
            self.wake();
        }
    }

    pub(crate) fn wake(&self) {
        if let Some(thread) = self.worker_thread.get() {
            thread.unpark();
        }
    }

    /// Queue a request for the worker. Returns it (dropping any reply
    /// sender, so a waiter sees `Disconnected` at once) when no worker will
    /// ever read it. The shutdown check sits under the `requests` lock, which
    /// the worker's final take also holds.
    pub(crate) fn request(&self, request: Request) -> bool {
        {
            let mut requests = lock(&self.requests);
            if self.shutdown.load(Ordering::Acquire) || !self.worker_alive.load(Ordering::Acquire) {
                return false;
            }
            requests.push(request);
        }
        self.wake();
        true
    }

    /// Worker exit, however it happened: refuse and drop pending requests,
    /// release the producers and the history, and wake every waiter.
    pub(crate) fn worker_exited(&self) {
        {
            let mut requests = lock(&self.requests);
            self.worker_alive.store(false, Ordering::Release);
            // A worker that died without being asked counts as shut down:
            // nothing will drain again, so stop registering producers.
            self.shutdown.store(true, Ordering::Release);
            // Dropping queued requests disconnects their reply channels.
            drop(std::mem::take(&mut *requests));
        }
        // After a panic the call sites must go quiet too, and a later
        // install must be able to take the slot.
        let me = self as *const Shared as *mut Shared;
        if GLOBAL
            .compare_exchange(
                me,
                std::ptr::null_mut(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            THRESHOLD.store(u8::MAX, Ordering::Relaxed);
            METRICS_ON.store(false, Ordering::Relaxed);
        }
        self.release_producers();
        // A panic poisons the state lock on the way out; the data is still
        // fine to drop.
        match self.state.try_lock() {
            Ok(mut state) => state.release_memory(),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().release_memory()
            }
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        let (done, signal) = &self.worker_done;
        *lock(done) = true;
        signal.notify_all();
    }

    /// Orphan every producer (their thread-locals drop them on next
    /// registration), fold their loss counters into the totals, and forget
    /// them. Idempotent.
    fn release_producers(&self) {
        let mut producers = lock(&self.producers);
        for producer in producers.iter() {
            producer.orphaned.store(true, Ordering::Release);
            self.stats
                .retired_events_dropped
                .fetch_add(producer.events.dropped(), Ordering::Relaxed);
            self.stats
                .retired_faults_dropped
                .fetch_add(producer.faults.dropped(), Ordering::Relaxed);
        }
        producers.clear();
    }
}

/// A diagnostics runtime: producer rings, flight recorder, and the worker
/// that persists them. Cheap to clone.
///
/// Most applications call [`install`] once and use the macros; a
/// `Diagnostics` value is for hosts and tests that want an instance of their
/// own.
#[derive(Clone)]
pub struct Diagnostics {
    pub(crate) shared: Arc<Shared>,
}

/// Counters describing what the runtime lost or wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiagnosticsStats {
    /// Events rejected because their thread's ring was full.
    pub events_dropped: u64,
    /// Faults rejected because their thread's emergency ring was full.
    pub faults_dropped: u64,
    /// Records lost because the thread was being torn down.
    pub lost_no_producer: u64,
    pub write_errors: u64,
    pub bytes_written: u64,
    /// Batches discarded while the session log was failing.
    pub bytes_discarded: u64,
    /// Two different descriptors claimed the same `(domain, id)`.
    pub schema_conflicts: u64,
    pub snapshots_written: u64,
    pub producers: usize,
}

impl Diagnostics {
    /// Start a runtime and its worker thread. With `config.enabled == false`
    /// no worker is spawned and everything is discarded.
    pub fn start(
        config: DiagnosticsConfig,
        meta: SessionMetadata,
        paths: DiagnosticsPaths,
    ) -> Self {
        let config = config.sanitized();
        let enabled = config.enabled;
        let min_severity = if enabled {
            config.min_severity as u8
        } else {
            u8::MAX
        };
        // One instant for both clocks: the header records where the session's
        // monotonic zero sits on the platform clock `Instant` reads.
        let start = Instant::now();
        let mut meta = meta;
        meta.monotonic_start_ns = crate::clock::monotonic_now_ns();
        let state = WorkerState::new(&config, &meta);
        let shared = Arc::new(Shared {
            id: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            min_severity,
            config,
            meta,
            paths,
            start,
            producers: Mutex::new(Vec::new()),
            next_thread: AtomicU32::new(1),
            state: Mutex::new(state),
            requests: Mutex::new(Vec::new()),
            worker: Mutex::new(None),
            worker_thread: OnceLock::new(),
            worker_alive: AtomicBool::new(false),
            worker_done: (Mutex::new(!enabled), Condvar::new()),
            shutdown: AtomicBool::new(!enabled),
            stats: Stats::default(),
        });
        if enabled {
            let worker_shared = shared.clone();
            // Alive before the thread exists, so a request sent right after
            // `start` is queued rather than refused.
            shared.worker_alive.store(true, Ordering::Release);
            match std::thread::Builder::new()
                .name("nana-diagnostics".into())
                .spawn(move || {
                    // Runs on normal exit and on panic alike.
                    struct Exit(Arc<Shared>);
                    impl Drop for Exit {
                        fn drop(&mut self) {
                            self.0.worker_exited();
                        }
                    }
                    let exit = Exit(worker_shared.clone());
                    worker::run(worker_shared);
                    drop(exit);
                }) {
                Ok(handle) => {
                    let _ = shared.worker_thread.set(handle.thread().clone());
                    *lock(&shared.worker) = Some(handle);
                }
                // No worker: rings fill and drop, producers never notice.
                // Snapshots still work from the calling thread.
                Err(_) => {
                    shared.stats.write_errors.fetch_add(1, Ordering::Relaxed);
                    shared.worker_alive.store(false, Ordering::Release);
                    *lock(&shared.worker_done.0) = true;
                }
            }
        }
        Self { shared }
    }

    /// Make this the process-wide instance the macros write to. Returns a
    /// guard that shuts the runtime down (final drain, durable flush) when
    /// dropped. Fails if another instance is installed.
    pub fn install_global(&self) -> Result<DiagnosticsGuard, AlreadyInstalled> {
        let raw = Arc::into_raw(self.shared.clone()) as *mut Shared;
        if GLOBAL
            .compare_exchange(
                std::ptr::null_mut(),
                raw,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            // SAFETY: `raw` came from `Arc::into_raw` just above.
            drop(unsafe { Arc::from_raw(raw) });
            return Err(AlreadyInstalled);
        }
        if self.shared.config.enabled {
            THRESHOLD.store(self.shared.min_severity, Ordering::Relaxed);
            METRICS_ON.store(self.shared.config.metrics, Ordering::Relaxed);
        }
        Ok(DiagnosticsGuard {
            diagnostics: self.clone(),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.shared.config.enabled
    }

    pub fn config(&self) -> &DiagnosticsConfig {
        &self.shared.config
    }

    pub fn metadata(&self) -> &SessionMetadata {
        &self.shared.meta
    }

    pub fn paths(&self) -> &DiagnosticsPaths {
        &self.shared.paths
    }

    /// Record an event on this instance (the macros use the global one).
    #[inline]
    pub fn emit(&self, event: &'static EventDescriptor, fields: &[Field]) {
        self.shared.emit(event, fields);
    }

    /// Record a fault. Faults use a separate emergency ring and wake the
    /// worker so they reach disk promptly.
    pub fn fault(&self, event: &'static EventDescriptor, fields: &[Field], message: Option<&str>) {
        if (event.severity as u8) < self.shared.min_severity {
            return;
        }
        self.shared.fault(event, fields, message.map(Into::into));
    }

    /// Pre-register the calling thread so its first event does not pay for
    /// registration. Real-time threads should call this at startup.
    pub fn register_current_thread(&self) {
        if self.shared.config.enabled {
            self.shared.with_producer(|_| {});
        }
    }

    /// Attach a late-known fact to the session (GPU adapter, backend...).
    /// Written to the log and repeated in every later file and snapshot.
    pub fn set_session_info(&self, key: impl Into<String>, value: impl Into<String>) {
        if self.shared.config.enabled {
            self.shared
                .request(Request::SessionInfo(key.into(), value.into()));
        }
    }

    /// A free-text marker. Cold path only; allocates.
    pub fn marker(&self, text: impl Into<String>) {
        if self.shared.config.enabled {
            self.shared.request(Request::Marker(text.into()));
        }
    }

    /// Ask the worker to write a flight-recorder snapshot to the crash
    /// directory. Returns immediately.
    pub fn snapshot(&self, reason: &str) {
        if self.shared.config.enabled {
            self.shared.request(Request::Snapshot {
                reason: reason.to_owned(),
                reply: None,
            });
        }
    }

    /// Write a flight-recorder snapshot and wait for its path. For explicit
    /// user exports; never call from a real-time thread.
    pub fn snapshot_blocking(&self, reason: &str, timeout: Duration) -> io::Result<PathBuf> {
        if !self.shared.config.enabled {
            return Err(io::Error::other("diagnostics are disabled"));
        }
        let (tx, rx) = mpsc::channel();
        self.shared.request(Request::Snapshot {
            reason: reason.to_owned(),
            reply: Some(tx),
        });
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            // No worker will answer: write it from here.
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                worker::snapshot_now(&self.shared, reason, None, timeout)
            }
            // The worker has it but is slow; writing a second copy here
            // would race it.
            Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "diagnostics worker did not finish the snapshot in time",
            )),
        }
    }

    /// Drain and write everything pending. `durable` also syncs to disk.
    /// Blocks up to `timeout`; returns whether the worker confirmed.
    pub fn flush(&self, durable: bool, timeout: Duration) -> bool {
        if !self.shared.config.enabled {
            return false;
        }
        let (tx, rx) = mpsc::channel();
        self.shared.request(Request::Flush {
            durable,
            reply: Some(tx),
        });
        rx.recv_timeout(timeout).is_ok()
    }

    /// Send the session log to `sink` instead of the logs directory. The
    /// sink's stream starts with a fresh header.
    pub fn set_sink(&self, sink: Box<dyn crate::files::Sink>) {
        if self.shared.config.enabled {
            self.shared.request(Request::Sink(sink));
        }
    }

    /// Stop the worker after a final drain and a durable flush. Idempotent.
    /// Waits at most [`SHUTDOWN_WAIT`] for the worker (a hung disk must not
    /// hang application exit); later events are discarded.
    pub fn shutdown(&self) {
        let shared = &self.shared;
        let me = Arc::as_ptr(shared) as *mut Shared;
        if GLOBAL
            .compare_exchange(
                me,
                std::ptr::null_mut(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            // The leaked reference keeps `Shared` valid for any producer that
            // loaded the pointer before this swap.
            THRESHOLD.store(u8::MAX, Ordering::Relaxed);
            METRICS_ON.store(false, Ordering::Relaxed);
        }
        {
            // Under the requests lock: see `Shared::request`.
            let _requests = lock(&shared.requests);
            shared.shutdown.store(true, Ordering::Release);
        }
        shared.wake();
        if !IS_WORKER.with(|flag| flag.get()) {
            // Every caller waits (concurrent shutdowns included), bounded so
            // a hung disk or sink cannot hang application exit.
            let (done, signal) = &shared.worker_done;
            let guard = lock(done);
            let (guard, _) = signal
                .wait_timeout_while(guard, SHUTDOWN_WAIT, |done| !*done)
                .unwrap_or_else(PoisonError::into_inner);
            let finished = *guard;
            drop(guard);
            // Unfinished: leave the handle for a later `shutdown`; the worker
            // frees its own state when (if) it gets there.
            if finished && let Some(handle) = lock(&shared.worker).take() {
                let _ = handle.join();
            }
        }
        // Covers the no-worker case; idempotent after the worker's own exit.
        shared.release_producers();
    }

    pub fn stats(&self) -> DiagnosticsStats {
        let shared = &self.shared;
        let producers = lock(&shared.producers);
        let s = &shared.stats;
        DiagnosticsStats {
            events_dropped: producers.iter().map(|p| p.events.dropped()).sum::<u64>()
                + s.retired_events_dropped.load(Ordering::Relaxed),
            faults_dropped: producers.iter().map(|p| p.faults.dropped()).sum::<u64>()
                + s.retired_faults_dropped.load(Ordering::Relaxed),
            lost_no_producer: s.lost_no_producer.load(Ordering::Relaxed),
            write_errors: s.write_errors.load(Ordering::Relaxed),
            bytes_written: s.bytes_written.load(Ordering::Relaxed),
            bytes_discarded: s.bytes_discarded.load(Ordering::Relaxed),
            schema_conflicts: s.schema_conflicts.load(Ordering::Relaxed),
            snapshots_written: s.snapshots_written.load(Ordering::Relaxed),
            producers: producers.len(),
        }
    }

    /// Path of the session log currently being written, if any.
    pub fn current_log(&self) -> Option<PathBuf> {
        lock(&self.shared.state).current_log.clone()
    }
}

/// Longest [`Diagnostics::shutdown`] waits for the worker's final flush.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyInstalled;

impl std::fmt::Display for AlreadyInstalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a global diagnostics instance is already installed")
    }
}

impl std::error::Error for AlreadyInstalled {}

/// Shuts the global instance down when dropped. Under `panic = "abort"` a
/// panic never runs this; the panic hook covers that path instead.
#[must_use = "dropping the guard shuts diagnostics down"]
pub struct DiagnosticsGuard {
    diagnostics: Diagnostics,
}

impl DiagnosticsGuard {
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
}

impl Drop for DiagnosticsGuard {
    fn drop(&mut self) {
        self.diagnostics.shutdown();
    }
}

/// Start a runtime, install it globally, and (per `config.panic_hook`) hook
/// panics. The usual entry point for applications.
pub fn install(
    config: DiagnosticsConfig,
    meta: SessionMetadata,
    paths: DiagnosticsPaths,
) -> Result<DiagnosticsGuard, AlreadyInstalled> {
    let panic_hook = config.enabled && config.panic_hook;
    let diagnostics = Diagnostics::start(config, meta, paths);
    let guard = match diagnostics.install_global() {
        Ok(guard) => guard,
        Err(e) => {
            diagnostics.shutdown();
            return Err(e);
        }
    };
    if panic_hook {
        crate::crash::install_panic_hook();
    }
    Ok(guard)
}

/// The installed global instance, if any.
pub fn global() -> Option<Diagnostics> {
    let raw = GLOBAL.load(Ordering::Acquire);
    if raw.is_null() {
        return None;
    }
    // SAFETY: GLOBAL only ever holds a pointer from `Arc::into_raw` whose
    // reference is never released, so the allocation is alive.
    unsafe { Arc::increment_strong_count(raw) };
    Some(Diagnostics {
        // SAFETY: we just added the strong reference this takes over.
        shared: unsafe { Arc::from_raw(raw) },
    })
}

#[inline]
fn with_global(f: impl FnOnce(&Shared)) {
    let raw = GLOBAL.load(Ordering::Acquire);
    // SAFETY: see `global`.
    if let Some(shared) = unsafe { raw.as_ref() } {
        f(shared);
    }
}

/// See [`Diagnostics::register_current_thread`].
pub fn register_thread() {
    with_global(|shared| shared.with_producer(|_| {}));
}

/// See [`Diagnostics::set_session_info`].
pub fn set_session_info(key: impl Into<String>, value: impl Into<String>) {
    if let Some(diagnostics) = global() {
        diagnostics.set_session_info(key, value);
    }
}

/// See [`Diagnostics::marker`].
pub fn marker(text: impl Into<String>) {
    if let Some(diagnostics) = global() {
        diagnostics.marker(text);
    }
}

/// See [`Diagnostics::snapshot`].
pub fn snapshot(reason: &str) {
    if let Some(diagnostics) = global() {
        diagnostics.snapshot(reason);
    }
}

#[doc(hidden)]
pub mod __private {
    use super::*;

    #[inline]
    pub fn emit(event: &'static EventDescriptor, fields: &[Field]) {
        with_global(|shared| shared.emit(event, fields));
    }

    #[cold]
    pub fn fault(event: &'static EventDescriptor, fields: &[Field], message: Option<Box<str>>) {
        with_global(|shared| shared.fault(event, fields, message));
    }
}
