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
use crate::record::FaultRecord;
use crate::record::saturating_ns;
use crate::runtime::{Producer, Shared, lock};
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
    /// The producer list as of `producers_generation`, so an idle tick does
    /// not clone it.
    producers: Vec<Arc<Producer>>,
    producers_generation: u64,
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
                window_ns: saturating_ns(config.flight_window),
                max_bytes: config.flight_bytes,
            },
            batch: Vec::new(),
            batch_since: None,
            urgent: false,
            producers: Vec::new(),
            producers_generation: u64::MAX,
            counter_last: HashMap::new(),
            gauge_last: HashMap::new(),
            dropped_last: (Vec::new(), Vec::new()),
            header: encode_header(meta, "session"),
            batch_is_complete: true,
            current_log: None,
        }
    }

    /// Append to the batch (when persisting) and start its age clock.
    fn batch_append(&mut self, bytes: &[u8]) {
        if self.persist == PersistMode::Off || bytes.is_empty() {
            return;
        }
        if self.batch.is_empty() {
            self.batch_since = Some(Instant::now());
        }
        self.batch.extend_from_slice(bytes);
    }

    /// Encode one chunk into the batch and the flight recorder.
    fn emit(&mut self, ts_ns: u64, encode: impl FnOnce(&mut ChunkWriter, &mut Vec<u8>)) {
        let mut chunk = Vec::new();
        encode(&mut self.writer, &mut chunk);
        self.batch_append(&chunk);
        self.flight.push(ts_ns, chunk);
    }

    /// Encode one schema-class chunk into the batch and every future prefix.
    fn emit_schema(&mut self, encode: impl FnOnce(&mut ChunkWriter, &mut Vec<u8>)) {
        let start = self.schema_bytes.len();
        encode(&mut self.writer, &mut self.schema_bytes);
        let chunk = self.schema_bytes[start..].to_vec();
        self.batch_append(&chunk);
    }

    fn schema(&mut self, shared: &Shared, event: &'static EventDescriptor) {
        let key = event.key();
        match self.events.get(&key) {
            Some(known) if std::ptr::eq(*known, event) => {}
            Some(known) => {
                if known.name != event.name || known.fields != event.fields {
                    shared
                        .stats
                        .schema_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            None => {
                self.events.insert(key, event);
                self.emit_schema(|w, out| w.event_schema(out, event));
            }
        }
    }

    fn fault(&mut self, shared: &Shared, fault: &FaultRecord) {
        self.schema(shared, fault.record.event);
        self.emit(fault.record.ts_ns, |w, out| w.fault(out, fault));
        self.urgent = true;
    }

    /// Pull everything out of every ring. Caller holds `Shared::state`,
    /// which makes this the only consumer.
    pub(crate) fn drain(&mut self, shared: &Shared) {
        // Refresh the list only when it changed, and drain without holding
        // `producers`: a thread registering its first event must never wait
        // on this (possibly descheduled, low-priority) worker.
        let generation = shared.producers_generation.load(Ordering::Acquire);
        if generation != self.producers_generation {
            self.producers = lock(&shared.producers).clone();
            self.producers_generation = generation;
        }
        let mut records = Vec::new();
        let mut faults = Vec::new();
        let mut retired_any = false;
        let producers = std::mem::take(&mut self.producers);
        for producer in &producers {
            // Read `retired` first: a retired producer pushes nothing more,
            // so draining after the load cannot miss a record.
            let retired = producer.retired.load(Ordering::Acquire);
            let before = records.len() + faults.len();
            // SAFETY: we hold `Shared::state`, the consumer lock.
            while let Some(record) = unsafe { producer.events.pop() } {
                records.push(record);
            }
            while let Some(fault) = unsafe { producer.faults.pop() } {
                faults.push(fault);
            }
            if records.len() + faults.len() > before && self.threads.insert(producer.thread) {
                self.emit_schema(|w, out| w.thread_name(out, producer.thread, &producer.name));
            }
            retired_any |= retired;
        }
        if retired_any {
            shared.forget_retired(&producers);
        }
        self.producers = producers;

        let mut previous: Option<&'static EventDescriptor> = None;
        for record in &records {
            // Events arrive in runs of the same descriptor.
            if !previous.is_some_and(|p| std::ptr::eq(p, record.event)) {
                self.schema(shared, record.event);
                previous = Some(record.event);
            }
        }
        if let Some(last) = records.iter().map(|r| r.ts_ns).max() {
            records.sort_by_key(|r| r.ts_ns);
            let mut all = Vec::new();
            self.writer.events(&mut all, &records);
            match self.persist {
                PersistMode::All => self.batch_append(&all),
                PersistMode::Essential => {
                    records.retain(|r| r.event.severity >= Severity::Warn);
                    let mut important = Vec::new();
                    self.writer.events(&mut important, &records);
                    self.batch_append(&important);
                }
                PersistMode::Off => {}
            }
            self.flight.push(last, all);
        }

        faults.sort_by_key(|f| f.record.ts_ns);
        for fault in &faults {
            self.fault(shared, fault);
        }
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
            if self.metrics.insert(key, descriptor).is_none() {
                self.emit_schema(|w, out| w.metric_schema(out, &descriptor));
            }
            items.push((descriptor, value));
        }
        if !items.is_empty() {
            self.emit(ts_ns, |w, out| w.metric_snapshot(out, ts_ns, &items));
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
        self.emit(ts_ns, |w, out| w.dropped(out, ts_ns, &threads, &stats));
        self.dropped_last = (threads, stats);
    }

    /// Free the flight recorder and buffers after shutdown.
    pub(crate) fn release_memory(&mut self) {
        self.flight.chunks = VecDeque::new();
        self.flight.bytes = 0;
        self.batch = Vec::new();
        self.producers = Vec::new();
    }

    /// Every session-info pair so far, as one chunk (empty when none).
    fn session_info_chunk(&mut self, out: &mut Vec<u8>, ts_ns: u64) {
        if !self.session_info.is_empty() {
            self.writer.session_info(out, ts_ns, &self.session_info);
        }
    }

    fn set_session_info(&mut self, ts_ns: u64, key: String, value: String) {
        let index = match self.session_info.iter().position(|(k, _)| *k == key) {
            Some(index) => {
                self.session_info[index].1 = value;
                index
            }
            None => {
                self.session_info.push((key, value));
                self.session_info.len() - 1
            }
        };
        let mut chunk = Vec::new();
        self.writer.session_info(
            &mut chunk,
            ts_ns,
            std::slice::from_ref(&self.session_info[index]),
        );
        // Not pushed to the flight recorder: snapshots carry the full set.
        self.batch_append(&chunk);
    }

    fn marker(&mut self, ts_ns: u64, text: &str) {
        self.emit(ts_ns, |w, out| w.marker(out, ts_ns, text));
    }

    /// Header, schemas, session info: what a fresh session-log file needs
    /// before the batch.
    fn file_prefix(&mut self, ts_ns: u64) -> Vec<u8> {
        // Until the first file is opened, every schema and session-info
        // chunk is still in the batch; repeating them would only duplicate.
        if std::mem::replace(&mut self.batch_is_complete, false) {
            return self.header.clone();
        }
        let mut out = self.header.clone();
        out.extend_from_slice(&self.schema_bytes);
        self.session_info_chunk(&mut out, ts_ns);
        out
    }

    /// A complete, self-describing snapshot file.
    fn snapshot_bytes(&mut self, meta: &SessionMetadata, reason: &str, ts_ns: u64) -> Vec<u8> {
        let header = encode_header(meta, &format!("snapshot:{reason}"));
        let mut out =
            Vec::with_capacity(header.len() + self.schema_bytes.len() + self.flight.bytes + 1024);
        out.extend_from_slice(&header);
        out.extend_from_slice(&self.schema_bytes);
        self.session_info_chunk(&mut out, ts_ns);
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
    crate::priority::lower_current_thread();
    let config = shared.config.clone();
    let meta = shared.meta.clone();
    let mut output = match (&shared.paths.logs, config.persist) {
        (Some(dir), mode) if mode != PersistMode::Off => Output::Files(RotatingLog::new(
            dir.clone(),
            &meta.app_id,
            shared.stem.clone(),
            config.retention.clone(),
        )),
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
                st.emit(ts, |w, out| {
                    w.clock_sync(out, ts, crate::session::unix_now_ns())
                });
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
    let name = format!("{}-{}", shared.stem, files::sanitize(reason));
    let result = files::write_new_file(dir, &name, bytes);
    match &result {
        Ok(path) => {
            shared
                .stats
                .snapshots_written
                .fetch_add(1, Ordering::Relaxed);
            let r = &shared.config.retention;
            files::prune(
                dir,
                &shared.meta.app_id,
                Some(&shared.stem),
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
    let is_worker = shared.on_worker_thread();
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
        state.fault(shared, &fault);
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
