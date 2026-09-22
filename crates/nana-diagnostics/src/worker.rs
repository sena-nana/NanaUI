//! The diagnostics worker: drains producer rings, aggregates metrics, keeps
//! the flight recorder, and writes the session log in batches.
//!
//! All encoder state lives in [`WorkerState`] behind `Shared::state`; holding
//! that lock makes the holder the single consumer of every ring. File I/O
//! happens with the lock released, so a slow disk never stalls a crash
//! snapshot that needs the state.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::files::{self, Backoff, RotatingLog, Sink};
use crate::metric::{self, Metric};
use crate::nlog::{ChunkWriter, encode_header, writer::SnapshotValue};
use crate::record::{FaultRecord, Record};
use crate::runtime::{IS_WORKER, Shared, lock};
use crate::schema::{EventDescriptor, MetricDescriptor, MetricKind, SchemaKey, Severity};
use crate::session::{DiagnosticsConfig, PersistMode, SessionMetadata};

pub(crate) enum Request {
    SessionInfo(String, String),
    Marker(String),
    Snapshot {
        reason: String,
        reply: Option<Sender<io::Result<PathBuf>>>,
    },
    Flush {
        durable: bool,
        reply: Option<Sender<()>>,
    },
    /// Replace the rotating log with a custom sink.
    Sink(Box<dyn Sink>),
}

/// Last written loss counters: per-thread `(thread, events, faults)` and
/// runtime-wide `(name, value)`.
type DropReport = (Vec<(u32, u64, u64)>, Vec<(&'static str, u64)>);

/// Recent encoded chunks, trimmed by age and total size.
struct FlightRecorder {
    chunks: VecDeque<(u64, Vec<u8>)>,
    bytes: usize,
    window_ns: u64,
    max_bytes: usize,
}

impl FlightRecorder {
    fn push(&mut self, ts_ns: u64, chunk: Vec<u8>) {
        if chunk.is_empty() {
            return;
        }
        self.bytes += chunk.len();
        self.chunks.push_back((ts_ns, chunk));
        let horizon = ts_ns.saturating_sub(self.window_ns);
        // The newest chunk always stays, even if it alone exceeds the budget.
        while self.chunks.len() > 1
            && let Some((ts, front)) = self.chunks.front()
        {
            if self.bytes <= self.max_bytes && *ts >= horizon {
                break;
            }
            self.bytes -= front.len();
            self.chunks.pop_front();
        }
    }
}

pub(crate) struct WorkerState {
    writer: ChunkWriter,
    persist: PersistMode,
    collect_metrics: bool,
    events: HashMap<SchemaKey, &'static EventDescriptor>,
    metrics: HashMap<SchemaKey, MetricDescriptor>,
    threads: HashSet<u32>,
    /// Every schema / thread-name chunk so far: the prefix of any new file.
    schema_bytes: Vec<u8>,
    session_info: Vec<(String, String)>,
    flight: FlightRecorder,
    /// Bytes waiting for the session log.
    batch: Vec<u8>,
    batch_since: Option<Instant>,
    /// Flush the batch at the end of this tick regardless of size/age.
    urgent: bool,
    records: Vec<Record>,
    faults: Vec<FaultRecord>,
    counter_last: HashMap<SchemaKey, u64>,
    gauge_last: HashMap<SchemaKey, u64>,
    dropped_last: DropReport,
    header: Vec<u8>,
    /// The batch still holds every schema / session-info chunk of the
    /// session (true until the first file takes a prefix).
    batch_is_complete: bool,
    pub(crate) current_log: Option<PathBuf>,
}

impl WorkerState {
    pub(crate) fn new(config: &DiagnosticsConfig, meta: &SessionMetadata) -> Self {
        Self {
            writer: ChunkWriter::default(),
            persist: config.persist,
            collect_metrics: config.metrics,
            events: HashMap::new(),
            metrics: HashMap::new(),
            threads: HashSet::new(),
            schema_bytes: Vec::new(),
            session_info: Vec::new(),
            flight: FlightRecorder {
                chunks: VecDeque::new(),
                bytes: 0,
                window_ns: u64::try_from(config.flight_window.as_nanos()).unwrap_or(u64::MAX),
                max_bytes: config.flight_bytes,
            },
            batch: Vec::new(),
            batch_since: None,
            urgent: false,
            records: Vec::new(),
            faults: Vec::new(),
            counter_last: HashMap::new(),
            gauge_last: HashMap::new(),
            dropped_last: (Vec::new(), Vec::new()),
            header: encode_header(meta, "session"),
            batch_is_complete: true,
            current_log: None,
        }
    }

    fn persists(&self) -> bool {
        self.persist != PersistMode::Off
    }

    /// Append to the batch (when persisting) and start its age clock.
    fn batch_append(&mut self, bytes: &[u8]) {
        if !self.persists() || bytes.is_empty() {
            return;
        }
        if self.batch.is_empty() {
            self.batch_since = Some(Instant::now());
        }
        self.batch.extend_from_slice(bytes);
    }

    fn schema(&mut self, shared: &Shared, event: &'static EventDescriptor) {
        let key = event.key();
        match self.events.get(&key) {
            Some(known) if std::ptr::eq(*known, event) => return,
            Some(known) => {
                if known.name != event.name || known.fields != event.fields {
                    shared
                        .stats
                        .schema_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                }
                return;
            }
            None => {}
        }
        self.events.insert(key, event);
        let mut chunk = Vec::new();
        self.writer.event_schema(&mut chunk, event);
        self.schema_bytes.extend_from_slice(&chunk);
        self.batch_append(&chunk);
    }

    fn metric_schema(&mut self, descriptor: MetricDescriptor) -> bool {
        if self.metrics.contains_key(&descriptor.key()) {
            return true;
        }
        self.metrics.insert(descriptor.key(), descriptor);
        let mut chunk = Vec::new();
        self.writer.metric_schema(&mut chunk, &descriptor);
        self.schema_bytes.extend_from_slice(&chunk);
        self.batch_append(&chunk);
        true
    }

    fn thread(&mut self, thread: u32, name: &str) {
        if self.threads.insert(thread) {
            let mut chunk = Vec::new();
            self.writer.thread_name(&mut chunk, thread, name);
            self.schema_bytes.extend_from_slice(&chunk);
            self.batch_append(&chunk);
        }
    }

    /// Pull everything out of every ring. Caller holds `Shared::state`,
    /// which makes this the only consumer.
    pub(crate) fn drain(&mut self, shared: &Shared) {
        // Snapshot the list and drain without holding `producers`: a thread
        // registering its first event must never wait on this (possibly
        // descheduled, low-priority) worker.
        let producers: Vec<Arc<crate::runtime::Producer>> = lock(&shared.producers).clone();
        self.records.clear();
        self.faults.clear();
        let mut retired_any = false;
        for producer in &producers {
            // Read `retired` first: a retired producer pushes nothing more,
            // so draining after the load cannot miss a record.
            let retired = producer.retired.load(Ordering::Acquire);
            let before = self.records.len() + self.faults.len();
            // SAFETY: we hold `Shared::state`, the consumer lock.
            while let Some(record) = unsafe { producer.events.pop() } {
                self.records.push(record);
            }
            while let Some(fault) = unsafe { producer.faults.pop() } {
                self.faults.push(fault);
            }
            if self.records.len() + self.faults.len() > before {
                self.thread(producer.thread, &producer.name);
            }
            retired_any |= retired;
        }
        if retired_any {
            let mut list = lock(&shared.producers);
            list.retain(|producer| {
                // Only drop what this pass saw retired *and* drained.
                let gone = producer.retired.load(Ordering::Acquire)
                    && producers.iter().any(|seen| Arc::ptr_eq(seen, producer))
                    && producer.events.is_empty()
                    && producer.faults.is_empty();
                if gone {
                    shared
                        .stats
                        .retired_events_dropped
                        .fetch_add(producer.events.dropped(), Ordering::Relaxed);
                    shared
                        .stats
                        .retired_faults_dropped
                        .fetch_add(producer.faults.dropped(), Ordering::Relaxed);
                }
                !gone
            });
        }

        let records = std::mem::take(&mut self.records);
        let faults = std::mem::take(&mut self.faults);
        for record in &records {
            self.schema(shared, record.event);
        }
        for fault in &faults {
            self.schema(shared, fault.record.event);
        }

        if !records.is_empty() {
            let mut sorted = records;
            sorted.sort_by_key(|r| r.ts_ns);
            let last_ts = sorted.last().map_or(0, |r| r.ts_ns);
            let mut all = Vec::new();
            self.writer.events(&mut all, &sorted);
            match self.persist {
                PersistMode::All => self.batch_append(&all),
                PersistMode::Essential => {
                    let important: Vec<Record> = sorted
                        .iter()
                        .copied()
                        .filter(|r| r.event.severity >= Severity::Warn)
                        .collect();
                    let mut chunk = Vec::new();
                    self.writer.events(&mut chunk, &important);
                    self.batch_append(&chunk);
                }
                PersistMode::Off => {}
            }
            self.flight.push(last_ts, all);
            sorted.clear();
            self.records = sorted;
        } else {
            self.records = records;
        }

        let mut faults = faults;
        faults.sort_by_key(|f| f.record.ts_ns);
        for fault in faults.drain(..) {
            let mut chunk = Vec::new();
            self.writer.fault(&mut chunk, &fault);
            self.batch_append(&chunk);
            self.flight.push(fault.record.ts_ns, chunk);
            self.urgent = true;
        }
        self.faults = faults;
    }

    fn snapshot_metrics(&mut self, ts_ns: u64) {
        if !self.collect_metrics {
            return;
        }
        let mut items = Vec::new();
        for metric in metric::registered() {
            let descriptor = metric.descriptor;
            let key = descriptor.key();
            let value = match descriptor.kind {
                MetricKind::Counter => {
                    let total = metric.value();
                    let last = self.counter_last.insert(key, total);
                    let delta = total.saturating_sub(last.unwrap_or(0));
                    if last.is_some() && delta == 0 {
                        continue;
                    }
                    SnapshotValue::Counter { total, delta }
                }
                MetricKind::Gauge => {
                    let value = metric.value();
                    if self.gauge_last.insert(key, value) == Some(value) {
                        continue;
                    }
                    SnapshotValue::Gauge(value)
                }
                MetricKind::Histogram => match Metric::take_histogram(metric) {
                    Some(sample) if sample.count > 0 => SnapshotValue::Histogram(sample),
                    _ => continue,
                },
            };
            self.metric_schema(descriptor);
            items.push((descriptor, value));
        }
        if !items.is_empty() {
            let mut chunk = Vec::new();
            self.writer.metric_snapshot(&mut chunk, ts_ns, &items);
            self.batch_append(&chunk);
            self.flight.push(ts_ns, chunk);
        }
    }

    /// Cumulative loss counters, written whenever they change.
    fn report_dropped(&mut self, shared: &Shared, ts_ns: u64) {
        let threads: Vec<(u32, u64, u64)> = lock(&shared.producers)
            .iter()
            .map(|p| (p.thread, p.events.dropped(), p.faults.dropped()))
            .filter(|(_, events, faults)| events + faults > 0)
            .collect();
        let s = &shared.stats;
        let stats: Vec<(&'static str, u64)> = [
            ("lost_no_producer", &s.lost_no_producer),
            ("write_errors", &s.write_errors),
            ("bytes_discarded", &s.bytes_discarded),
            ("schema_conflicts", &s.schema_conflicts),
            ("retired_events_dropped", &s.retired_events_dropped),
            ("retired_faults_dropped", &s.retired_faults_dropped),
        ]
        .into_iter()
        .map(|(name, value)| (name, value.load(Ordering::Relaxed)))
        .filter(|(_, value)| *value > 0)
        .collect();
        if (&threads, &stats) == (&self.dropped_last.0, &self.dropped_last.1) {
            return;
        }
        let mut chunk = Vec::new();
        self.writer.dropped(&mut chunk, ts_ns, &threads, &stats);
        self.batch_append(&chunk);
        self.flight.push(ts_ns, chunk);
        self.dropped_last = (threads, stats);
    }

    /// Free the flight recorder and buffers after shutdown.
    pub(crate) fn release_memory(&mut self) {
        self.flight.chunks = VecDeque::new();
        self.flight.bytes = 0;
        self.batch = Vec::new();
        self.records = Vec::new();
        self.faults = Vec::new();
    }

    fn session_info_chunk(&mut self, ts_ns: u64) -> Vec<u8> {
        let mut chunk = Vec::new();
        if !self.session_info.is_empty() {
            let pairs = self.session_info.clone();
            self.writer.session_info(&mut chunk, ts_ns, &pairs);
        }
        chunk
    }

    fn set_session_info(&mut self, ts_ns: u64, key: String, value: String) {
        match self.session_info.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value.clone(),
            None => self.session_info.push((key.clone(), value.clone())),
        }
        let mut chunk = Vec::new();
        self.writer.session_info(&mut chunk, ts_ns, &[(key, value)]);
        self.batch_append(&chunk);
        // Not pushed to the flight recorder: snapshots carry the full set.
    }

    fn marker(&mut self, ts_ns: u64, text: &str) {
        let mut chunk = Vec::new();
        self.writer.marker(&mut chunk, ts_ns, text);
        self.batch_append(&chunk);
        self.flight.push(ts_ns, chunk);
    }

    fn clock_sync(&mut self, ts_ns: u64) {
        let mut chunk = Vec::new();
        self.writer
            .clock_sync(&mut chunk, ts_ns, crate::session::unix_now_ns());
        self.batch_append(&chunk);
        self.flight.push(ts_ns, chunk);
    }

    /// Header, schemas, session info: what a fresh session-log file needs
    /// before the batch.
    fn file_prefix(&mut self, ts_ns: u64) -> Vec<u8> {
        // Until the first file is opened, every schema and session-info
        // chunk is still in the batch; repeating them would only duplicate.
        if std::mem::replace(&mut self.batch_is_complete, false) {
            return self.header.clone();
        }
        let info = self.session_info_chunk(ts_ns);
        let mut out = Vec::with_capacity(self.header.len() + self.schema_bytes.len() + info.len());
        out.extend_from_slice(&self.header);
        out.extend_from_slice(&self.schema_bytes);
        out.extend_from_slice(&info);
        out
    }

    /// A complete, self-describing snapshot file.
    fn snapshot_bytes(&mut self, meta: &SessionMetadata, reason: &str, ts_ns: u64) -> Vec<u8> {
        let mut out = encode_header(meta, &format!("snapshot:{reason}"));
        out.extend_from_slice(&self.schema_bytes);
        let info = self.session_info_chunk(ts_ns);
        out.extend_from_slice(&info);
        for (_, chunk) in &self.flight.chunks {
            out.extend_from_slice(chunk);
        }
        self.writer
            .marker(&mut out, ts_ns, &format!("snapshot: {reason}"));
        out
    }

    fn take_batch(&mut self) -> Vec<u8> {
        self.batch_since = None;
        self.urgent = false;
        std::mem::take(&mut self.batch)
    }
}

enum Output {
    None,
    Files(RotatingLog),
    Custom { sink: Box<dyn Sink>, started: bool },
}

impl Output {
    fn needs_prefix(&self, incoming: usize) -> bool {
        match self {
            Self::None => false,
            Self::Files(log) => log.needs_prefix(incoming),
            Self::Custom { started, .. } => !started,
        }
    }

    fn write(&mut self, prefix: Option<&[u8]>, batch: &[u8]) -> io::Result<()> {
        match self {
            Self::None => Ok(()),
            Self::Files(log) => log.write(prefix, batch),
            Self::Custom { sink, started } => {
                if let Some(prefix) = prefix {
                    let mut first = prefix.to_vec();
                    first.extend_from_slice(batch);
                    sink.write(&first)?;
                    *started = true;
                    Ok(())
                } else {
                    sink.write(batch)
                }
            }
        }
    }

    fn reset(&mut self) {
        match self {
            Self::None => {}
            Self::Files(log) => log.reset(),
            Self::Custom { started, .. } => *started = false,
        }
    }

    fn flush(&mut self, durable: bool) -> io::Result<()> {
        match self {
            Self::None => Ok(()),
            Self::Files(log) => log.flush(durable),
            Self::Custom { sink, .. } => sink.flush(durable),
        }
    }

    fn path(&self) -> Option<PathBuf> {
        match self {
            Self::Files(log) => log.current_path().map(Into::into),
            _ => None,
        }
    }
}

/// Run a sink operation; a panicking custom sink is replaced by no output
/// instead of taking the worker down with it.
fn guarded(output: &mut Output, op: impl FnOnce(&mut Output) -> io::Result<()>) -> io::Result<()> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| op(output))) {
        Ok(result) => result,
        Err(_) => {
            *output = Output::None;
            Err(io::Error::other("diagnostics sink panicked"))
        }
    }
}

pub(crate) fn run(shared: Arc<Shared>) {
    IS_WORKER.with(|flag| flag.set(true));
    crate::priority::lower_current_thread();
    let config = shared.config.clone();
    let meta = shared.meta.clone();
    let mut output = match (&shared.paths.logs, config.persist) {
        (Some(dir), mode) if mode != PersistMode::Off => {
            let stem = files::session_stem(&meta.app_id, meta.wall_start_unix_ns, meta.pid);
            Output::Files(RotatingLog::new(
                dir.clone(),
                &meta.app_id,
                stem,
                config.retention.clone(),
            ))
        }
        _ => Output::None,
    };
    if let Some(dir) = &shared.paths.crash {
        files::prune(
            dir,
            &meta.app_id,
            None,
            None,
            config.retention.max_crash_files,
            config.retention.max_crash_bytes,
            &config.retention,
        );
    }
    let mut backoff = Backoff::new();
    let start = Instant::now();
    // Intervals are clamped by `DiagnosticsConfig::sanitized`, so these
    // additions cannot overflow.
    let mut next_metrics = start + config.metric_interval;
    let mut next_clock = start;
    let mut replies: Vec<Sender<()>> = Vec::new();

    loop {
        std::thread::park_timeout(config.poll_interval);
        // Every `Diagnostics` handle is gone: nobody can ask for a shutdown,
        // so do it now rather than poll forever. The two references left are
        // this loop's and the exit guard's (see `Diagnostics::start`).
        if Arc::strong_count(&shared) <= 2 {
            shared.shutdown.store(true, Ordering::Release);
        }
        let shutting_down = shared.shutdown.load(Ordering::Acquire);
        let requests = std::mem::take(&mut *lock(&shared.requests));
        let now = Instant::now();
        let mut snapshots = Vec::new();
        let mut durable = shutting_down;

        let (batch, prefix) = {
            let mut st = lock(&shared.state);
            st.drain(&shared);
            let ts = shared.now_ns();
            if now >= next_clock {
                st.clock_sync(ts);
                next_clock = now + config.clock_sync_interval;
            }
            if now >= next_metrics || shutting_down {
                st.snapshot_metrics(ts);
                st.report_dropped(&shared, ts);
                next_metrics = now + config.metric_interval;
            }
            for request in requests {
                match request {
                    Request::SessionInfo(key, value) => st.set_session_info(ts, key, value),
                    Request::Marker(text) => st.marker(ts, &text),
                    Request::Snapshot { reason, reply } => {
                        // The snapshot file ends with its own marker; the
                        // session log gets one so the two can be matched.
                        let mut chunk = Vec::new();
                        st.writer
                            .marker(&mut chunk, ts, &format!("snapshot: {reason}"));
                        st.batch_append(&chunk);
                        let bytes = st.snapshot_bytes(&meta, &reason, ts);
                        snapshots.push((reason, bytes, reply));
                        st.urgent = true;
                    }
                    Request::Flush {
                        durable: want_durable,
                        reply,
                    } => {
                        durable |= want_durable;
                        st.urgent = true;
                        replies.extend(reply);
                    }
                    Request::Sink(sink) => {
                        output = Output::Custom {
                            sink,
                            started: false,
                        };
                    }
                }
            }
            if shutting_down {
                st.marker(ts, "shutdown");
            }
            let due = st.urgent
                || shutting_down
                || st.batch.len() >= config.batch_bytes
                || st
                    .batch_since
                    .is_some_and(|since| now.duration_since(since) >= config.batch_interval);
            if due && !st.batch.is_empty() {
                let batch = st.take_batch();
                let prefix = output.needs_prefix(batch.len()).then(|| st.file_prefix(ts));
                if matches!(output, Output::None) {
                    // This batch (and the schemas in it) goes nowhere, so a
                    // sink attached later needs the full prefix.
                    st.batch_is_complete = false;
                }
                (Some(batch), prefix)
            } else {
                st.urgent = false;
                (None, None)
            }
        };

        for (reason, bytes, reply) in snapshots {
            let result = write_snapshot(&shared, &reason, &bytes);
            if let Some(reply) = reply {
                let _ = reply.send(result);
            }
        }

        if let Some(batch) = batch {
            if matches!(output, Output::None) {
                // Nothing to write to; the flight recorder still has it.
            } else if backoff.blocked(now) {
                shared
                    .stats
                    .bytes_discarded
                    .fetch_add(batch.len() as u64, Ordering::Relaxed);
            } else {
                let written = prefix.as_ref().map_or(0, Vec::len) + batch.len();
                match guarded(&mut output, |output| {
                    output.write(prefix.as_deref(), &batch)
                }) {
                    Ok(()) => {
                        backoff.succeeded();
                        shared
                            .stats
                            .bytes_written
                            .fetch_add(written as u64, Ordering::Relaxed);
                        if prefix.is_some() {
                            lock(&shared.state).current_log = output.path();
                        }
                    }
                    Err(_) => {
                        backoff.failed(now);
                        output.reset();
                        shared.stats.write_errors.fetch_add(1, Ordering::Relaxed);
                        shared
                            .stats
                            .bytes_discarded
                            .fetch_add(batch.len() as u64, Ordering::Relaxed);
                    }
                }
            }
        }
        if (durable || !replies.is_empty())
            && guarded(&mut output, |output| output.flush(durable)).is_err()
        {
            shared.stats.write_errors.fetch_add(1, Ordering::Relaxed);
        }
        for reply in replies.drain(..) {
            let _ = reply.send(());
        }
        if shutting_down {
            break;
        }
    }
    // Pending requests, producers and history are released by the exit
    // guard in `Diagnostics::start`, which also runs if this panics.
}

fn write_snapshot(shared: &Shared, reason: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let Some(dir) = &shared.paths.crash else {
        return Err(io::Error::other("no crash directory configured"));
    };
    let meta = &shared.meta;
    let name = format!(
        "{}-{}",
        files::session_stem(&meta.app_id, meta.wall_start_unix_ns, meta.pid),
        files::sanitize(reason)
    );
    let result = files::write_new_file(dir, &name, bytes);
    match &result {
        Ok(path) => {
            shared
                .stats
                .snapshots_written
                .fetch_add(1, Ordering::Relaxed);
            let r = &shared.config.retention;
            let stem = files::session_stem(&meta.app_id, meta.wall_start_unix_ns, meta.pid);
            files::prune(
                dir,
                &meta.app_id,
                Some(&stem),
                Some(path),
                r.max_crash_files,
                r.max_crash_bytes,
                r,
            );
        }
        Err(_) => {
            shared.stats.write_errors.fetch_add(1, Ordering::Relaxed);
        }
    }
    result
}

/// Write a snapshot from the calling thread (panic hook, or a blocking
/// snapshot whose worker did not answer). Waits up to `wait` for the state
/// lock; `fault` is recorded into the flight recorder first.
pub(crate) fn snapshot_now(
    shared: &Shared,
    reason: &str,
    fault: Option<FaultRecord>,
    wait: Duration,
) -> io::Result<PathBuf> {
    // `wait` may be `Duration::MAX` ("as long as it takes").
    let deadline = Instant::now().checked_add(wait);
    let is_worker = IS_WORKER.with(|flag| flag.get());
    let mut state = loop {
        match shared.state.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => break poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                // The worker may be the thread that is panicking while it
                // holds the lock: waiting would never succeed.
                if is_worker || deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "diagnostics state is busy",
                    ));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    };
    state.drain(shared);
    let ts = shared.now_ns();
    if let Some(fault) = fault {
        state.schema(shared, fault.record.event);
        let mut chunk = Vec::new();
        state.writer.fault(&mut chunk, &fault);
        state.batch_append(&chunk);
        state.flight.push(fault.record.ts_ns, chunk);
        state.urgent = true;
    }
    state.snapshot_metrics(ts);
    state.report_dropped(shared, ts);
    let bytes = state.snapshot_bytes(&shared.meta, reason, ts);
    drop(state);
    let result = write_snapshot(shared, reason, &bytes);
    shared.wake();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flight_recorder_trims_by_age_and_bytes() {
        let mut flight = FlightRecorder {
            chunks: VecDeque::new(),
            bytes: 0,
            window_ns: 1_000,
            max_bytes: 10,
        };
        flight.push(100, vec![0; 4]);
        flight.push(200, vec![0; 4]);
        assert_eq!(flight.chunks.len(), 2);
        // Too many bytes: the oldest goes.
        flight.push(300, vec![0; 4]);
        assert_eq!(flight.chunks.len(), 2);
        assert_eq!(flight.bytes, 8);
        // Too old: everything before ts 1_500 - 1_000 goes.
        flight.push(1_500, vec![0; 1]);
        assert_eq!(
            flight.chunks.iter().map(|(ts, _)| *ts).collect::<Vec<_>>(),
            [1_500]
        );
    }
}
