//! The diagnostics runtime: producer registration, the emit path, the
//! process-wide default instance, and the control API.

use std::cell::RefCell;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{
    AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::marker::{RunMarker, UncleanExit};
use crate::record::{FaultRecord, Field, Record, pack, saturating_ns};
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

const NB_SNAP_IDLE: u8 = 0;
const NB_SNAP_WRITING: u8 = 1;
const NB_SNAP_READY: u8 = 2;
const NB_SNAP_READING: u8 = 3;

/// Lock-free, allocation-free coalescing slot for non-blocking snapshot
/// triggers from latency-critical threads.
pub(crate) struct NonblockingSnapshotSlot {
    state: AtomicU8,
    reason_ptr: AtomicPtr<u8>,
    reason_len: AtomicUsize,
    /// Monotonic timestamp (`now_ns + 1`, `0` = never queued) of the last
    /// accepted non-blocking snapshot request, for coalescing/throttling.
    last_queued_ns: AtomicU64,
}

impl NonblockingSnapshotSlot {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(NB_SNAP_IDLE),
            reason_ptr: AtomicPtr::new(std::ptr::null_mut()),
            reason_len: AtomicUsize::new(0),
            last_queued_ns: AtomicU64::new(0),
        }
    }

    /// Try to enqueue `reason` without locking or allocating. Returns `false`
    /// if a request is already pending or `min_interval` has not elapsed since
    /// the last accepted non-blocking request.
    fn try_enqueue(&self, now_ns: u64, reason: &'static str, min_interval: Duration) -> bool {
        let min_ns = saturating_ns(min_interval);
        let stamp = now_ns.saturating_add(1);
        let prev = self.last_queued_ns.load(Ordering::Acquire);
        if prev != 0 && stamp.saturating_sub(prev) < min_ns {
            return false;
        }
        if self
            .state
            .compare_exchange(
                NB_SNAP_IDLE,
                NB_SNAP_WRITING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }
        let prev = self.last_queued_ns.load(Ordering::Acquire);
        if prev != 0 && stamp.saturating_sub(prev) < min_ns {
            self.state.store(NB_SNAP_IDLE, Ordering::Release);
            return false;
        }
        self.last_queued_ns.store(stamp, Ordering::Release);
        self.reason_ptr
            .store(reason.as_ptr().cast_mut(), Ordering::Relaxed);
        self.reason_len.store(reason.len(), Ordering::Relaxed);
        self.state.store(NB_SNAP_READY, Ordering::Release);
        true
    }

    /// Take the pending non-blocking snapshot reason, if any. Called by the
    /// worker thread.
    pub(crate) fn take(&self) -> Option<&'static str> {
        if self
            .state
            .compare_exchange(
                NB_SNAP_READY,
                NB_SNAP_READING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return None;
        }
        let ptr = self.reason_ptr.load(Ordering::Relaxed);
        let len = self.reason_len.load(Ordering::Relaxed);
        self.state.store(NB_SNAP_IDLE, Ordering::Release);
        // SAFETY: `ptr` and `len` came from a valid `&'static str` stored while
        // the producer held `NB_SNAP_WRITING` before its `Release` store of
        // `NB_SNAP_READY`. Our `AcqRel` CAS synchronized with that store, and
        // no producer can overwrite the fields while state is `NB_SNAP_READING`.
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        std::str::from_utf8(bytes).ok()
    }

    fn clear(&self) {
        let _ = self.take();
    }
}

pub(crate) struct Shared {
    id: u64,
    pub(crate) config: DiagnosticsConfig,
    pub(crate) meta: SessionMetadata,
    pub(crate) paths: DiagnosticsPaths,
    /// `{app}-{stamp}-{pid}`: the session log's and its snapshots' name.
    pub(crate) stem: String,
    start: Instant,
    min_severity: u8,
    pub(crate) producers: Mutex<Vec<Arc<Producer>>>,
    /// Bumped whenever `producers` changes, so the worker re-reads it only
    /// then.
    pub(crate) producers_generation: AtomicU64,
    next_thread: AtomicU32,
    /// Consumer side of every ring plus all encoder state. Whoever holds it
    /// is the one consumer the SPSC rings allow.
    pub(crate) state: Mutex<WorkerState>,
    pub(crate) requests: Mutex<Vec<Request>>,
    /// Lock-free single-slot trigger for hot-path non-blocking snapshots.
    pub(crate) nonblocking_snapshot: NonblockingSnapshotSlot,
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
    /// This session's run marker; taken (and deleted) by a clean shutdown.
    pub(crate) marker: Mutex<Option<RunMarker>>,
    /// Earlier sessions found without a clean shutdown at start.
    unclean_exits: Vec<UncleanExit>,
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
}

impl Stats {
    /// Fold a producer's loss counters into the totals before forgetting it.
    fn absorb(&self, producer: &Producer) {
        self.retired_events_dropped
            .fetch_add(producer.events.dropped(), Ordering::Relaxed);
        self.retired_faults_dropped
            .fetch_add(producer.faults.dropped(), Ordering::Relaxed);
    }
}

impl Shared {
    #[inline]
    pub(crate) fn now_ns(&self) -> u64 {
        saturating_ns(self.start.elapsed())
    }

    /// Whether the caller is this instance's worker thread.
    pub(crate) fn on_worker_thread(&self) -> bool {
        self.worker_thread
            .get()
            .is_some_and(|thread| thread.id() == std::thread::current().id())
    }

    /// Clear the global slot if it holds this instance, and silence every
    /// call site. The leaked reference keeps `Shared` valid for producers
    /// that loaded the pointer before the swap.
    fn uninstall_global(&self) {
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
    }

    /// Forget retired producers the worker saw in `drained` and emptied.
    pub(crate) fn forget_retired(&self, drained: &[Arc<Producer>]) {
        let mut list = lock(&self.producers);
        list.retain(|producer| {
            let gone = producer.retired.load(Ordering::Acquire)
                && drained.iter().any(|seen| Arc::ptr_eq(seen, producer))
                && producer.events.is_empty()
                && producer.faults.is_empty();
            if gone {
                self.stats.absorb(producer);
            }
            !gone
        });
        self.producers_generation.fetch_add(1, Ordering::Release);
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
            self.producers_generation.fetch_add(1, Ordering::Release);
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
        if (event.severity as u8) < self.min_severity {
            return;
        }
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

    /// Lock-free, allocation-free non-blocking snapshot trigger with
    /// coalescing and throttling. Safe on latency-critical / real-time paths.
    pub(crate) fn request_snapshot_nonblocking(
        &self,
        reason: &'static str,
        min_interval: Duration,
    ) -> bool {
        if !self.config.enabled
            || self.shutdown.load(Ordering::Acquire)
            || !self.worker_alive.load(Ordering::Acquire)
        {
            return false;
        }
        if self
            .nonblocking_snapshot
            .try_enqueue(self.now_ns(), reason, min_interval)
        {
            self.wake();
            true
        } else {
            false
        }
    }

    /// Worker exit, however it happened: refuse and drop pending requests,
    /// release the producers and the history, and wake every waiter.
    pub(crate) fn worker_exited(&self) {
        self.nonblocking_snapshot.clear();
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
        self.uninstall_global();
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
            self.stats.absorb(producer);
        }
        producers.clear();
        self.producers_generation.fetch_add(1, Ordering::Release);
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
        let stem = crate::files::session_stem(&meta.app_id, meta.wall_start_unix_ns, meta.pid);
        // Earlier sessions' markers first, then this one's.
        let marker_dir = (enabled && config.run_marker)
            .then(|| paths.crash.clone().or_else(|| paths.logs.clone()))
            .flatten();
        let unclean_exits = marker_dir.as_deref().map_or_else(Vec::new, |dir| {
            crate::marker::take_stale(dir, &meta.app_id, paths.logs.as_deref())
        });
        let marker = marker_dir
            .as_deref()
            .and_then(|dir| RunMarker::create(dir, &stem, &meta).ok());
        let shared = Arc::new(Shared {
            id: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            min_severity,
            config,
            meta,
            paths,
            stem,
            start,
            producers: Mutex::new(Vec::new()),
            producers_generation: AtomicU64::new(0),
            next_thread: AtomicU32::new(1),
            state: Mutex::new(state),
            requests: Mutex::new(Vec::new()),
            nonblocking_snapshot: NonblockingSnapshotSlot::new(),
            worker: Mutex::new(None),
            worker_thread: OnceLock::new(),
            worker_alive: AtomicBool::new(false),
            worker_done: (Mutex::new(!enabled), Condvar::new()),
            shutdown: AtomicBool::new(!enabled),
            stats: Stats::default(),
            marker: Mutex::new(marker),
            unclean_exits,
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
        let diagnostics = Self { shared };
        diagnostics.report_unclean_exits();
        diagnostics
    }

    /// One fault per earlier session that left its marker, and (unless each
    /// of them wrote its own crash snapshot) an `unclean-exit` snapshot,
    /// written before `start` returns so [`Diagnostics::last_crash`] sees it
    /// at once.
    fn report_unclean_exits(&self) {
        let shared = &self.shared;
        if shared.unclean_exits.is_empty() {
            return;
        }
        for exit in &shared.unclean_exits {
            shared.fault(
                &crate::framework::diagnostics::UNCLEAN_EXIT,
                &[
                    Field::new("session_id", exit.session_id),
                    Field::new("pid", exit.pid),
                    Field::new("start_unix_ns", exit.start_unix_ns),
                    Field::new("last_write_unix_ns", exit.last_write_unix_ns),
                ],
                Some(exit.describe().into_boxed_str()),
            );
        }
        if shared
            .unclean_exits
            .iter()
            .any(|exit| exit.crash_snapshot.is_none())
        {
            let _ = worker::snapshot_now(
                shared,
                crate::crash::UNCLEAN_EXIT_REASON,
                None,
                SYNC_SNAPSHOT_WAIT,
            );
        }
    }

    /// Earlier sessions of this app that ended without shutting down, found
    /// (and reported as `diagnostics.unclean_exit` faults) at start. Empty
    /// when `config.run_marker` is off.
    pub fn unclean_exits(&self) -> &[UncleanExit] {
        &self.shared.unclean_exits
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

    /// Lock-free, allocation-free snapshot trigger for latency-critical
    /// threads (display, capture, audio).
    ///
    /// Returns `true` if a snapshot was queued for the worker, or `false` if
    /// diagnostics are disabled, a non-blocking snapshot is already pending
    /// (coalesced), or `min_interval` has not yet elapsed since the previous
    /// accepted non-blocking request.
    pub fn request_snapshot_nonblocking(
        &self,
        reason: &'static str,
        min_interval: Duration,
    ) -> bool {
        self.shared
            .request_snapshot_nonblocking(reason, min_interval)
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

    /// Record a fault and write a flight-recorder snapshot from the calling
    /// thread before returning, for faults the process may not outlive (a
    /// lost GPU device) where the worker's next poll could come too late.
    /// Waits up to `wait` for the worker's state; blocks on disk. Never call
    /// it on a per-frame path.
    pub fn fault_with_snapshot(
        &self,
        event: &'static EventDescriptor,
        fields: &[Field],
        message: Option<&str>,
        reason: &str,
        wait: Duration,
    ) -> io::Result<PathBuf> {
        let shared = &self.shared;
        if !shared.config.enabled || (event.severity as u8) < shared.min_severity {
            return Err(io::Error::other("diagnostics are disabled"));
        }
        let (len, values) = pack(event, fields);
        let fault = FaultRecord {
            record: Record {
                ts_ns: shared.now_ns(),
                event,
                thread: 0,
                len,
                values,
            },
            message: message.map(Into::into),
        };
        worker::snapshot_now(shared, reason, Some(fault), wait)
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
        shared.uninstall_global();
        {
            // Under the requests lock: see `Shared::request`.
            let _requests = lock(&shared.requests);
            shared.shutdown.store(true, Ordering::Release);
        }
        shared.wake();
        if !shared.on_worker_thread() {
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
        // A clean exit, even if a hung disk kept the worker from finishing.
        if let Some(marker) = lock(&shared.marker).take() {
            marker.remove();
        }
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

/// How long a synchronous snapshot taken outside the worker waits for its
/// state lock.
pub(crate) const SYNC_SNAPSHOT_WAIT: Duration = Duration::from_secs(1);

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
    // Cheap early answer; `install_global` below settles any race.
    if !GLOBAL.load(Ordering::Acquire).is_null() {
        return Err(AlreadyInstalled);
    }
    let panic_hook = config.enabled && config.panic_hook;
    let native_crash_handler = config.enabled && config.native_crash_handler;
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
    if native_crash_handler {
        crate::native_crash::install();
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

/// [`Diagnostics::fault_with_snapshot`] on the global instance, waiting up to
/// one second for its state. Fails when none is installed.
pub fn fault_with_snapshot(
    event: &'static EventDescriptor,
    fields: &[Field],
    message: Option<&str>,
    reason: &str,
) -> io::Result<PathBuf> {
    match global() {
        Some(diagnostics) => {
            diagnostics.fault_with_snapshot(event, fields, message, reason, SYNC_SNAPSHOT_WAIT)
        }
        None => Err(io::Error::other("no diagnostics instance is installed")),
    }
}

/// See [`Diagnostics::snapshot`].
pub fn snapshot(reason: &str) {
    if let Some(diagnostics) = global() {
        diagnostics.snapshot(reason);
    }
}

/// See [`Diagnostics::request_snapshot_nonblocking`].
///
/// Lock-free and allocation-free: does not touch the request mutex or bump the
/// global `Arc` reference count.
pub fn request_snapshot_nonblocking(reason: &'static str, min_interval: Duration) -> bool {
    let raw = GLOBAL.load(Ordering::Acquire);
    // SAFETY: GLOBAL only ever holds a pointer from `Arc::into_raw` whose
    // reference is never released, so the allocation is alive.
    match unsafe { raw.as_ref() } {
        Some(shared) => shared.request_snapshot_nonblocking(reason, min_interval),
        None => false,
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use super::{Diagnostics, NonblockingSnapshotSlot};
    use crate::{DiagnosticsConfig, DiagnosticsPaths, SessionMetadata};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(1);

    fn temp_dir(name: &str) -> PathBuf {
        let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("nana-diag-rt-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn nonblocking_snapshot_slot_coalesces_and_throttles() {
        let slot = NonblockingSnapshotSlot::new();
        let interval = Duration::from_secs(5);

        assert!(slot.try_enqueue(0, "first", interval));
        // Second enqueue while pending coalesces even with zero interval.
        assert!(!slot.try_enqueue(10_000_000_000, "second", Duration::ZERO));
        assert_eq!(slot.take(), Some("first"));
        assert_eq!(slot.take(), None);

        // Throttled within min_interval.
        assert!(!slot.try_enqueue(1_000_000_000, "too-soon", interval));
        // Allowed once min_interval elapses.
        assert!(slot.try_enqueue(5_000_000_000, "after-window", interval));
        assert_eq!(slot.take(), Some("after-window"));
    }

    #[test]
    fn request_snapshot_nonblocking_writes_snapshot_and_respects_throttle() {
        let root = temp_dir("nb-snap");
        let crash = root.join("crash");
        let diag = Diagnostics::start(
            DiagnosticsConfig {
                poll_interval: Duration::from_millis(10),
                ..DiagnosticsConfig::default()
            },
            SessionMetadata::new("dev.rt.nb", "RtNb", "1.0.0"),
            DiagnosticsPaths {
                logs: None,
                crash: Some(crash.clone()),
            },
        );

        assert!(diag.request_snapshot_nonblocking("frame-stall", Duration::from_secs(60)));
        // Immediate repeat is coalesced/throttled.
        assert!(!diag.request_snapshot_nonblocking("frame-stall", Duration::from_secs(60)));

        let deadline = Instant::now() + Duration::from_secs(5);
        while diag.stats().snapshots_written == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(diag.stats().snapshots_written, 1);

        // Still inside the 60s throttle window after worker drained it.
        assert!(!diag.request_snapshot_nonblocking("frame-stall", Duration::from_secs(60)));
        // Zero interval bypasses throttle and queues another snapshot.
        assert!(diag.request_snapshot_nonblocking("follow-up", Duration::ZERO));

        let deadline = Instant::now() + Duration::from_secs(5);
        while diag.stats().snapshots_written < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(diag.stats().snapshots_written, 2);

        let files: Vec<String> = fs::read_dir(&crash)
            .expect("read crash dir")
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name.ends_with(".nlog"))
            .collect();
        assert!(files.iter().any(|n| n.contains("frame-stall")));
        assert!(files.iter().any(|n| n.contains("follow-up")));

        diag.shutdown();
        let _ = fs::remove_dir_all(&root);
    }
}
