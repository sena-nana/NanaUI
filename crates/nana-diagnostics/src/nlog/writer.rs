//! Chunk encoders used by the worker and the crash path.

use crate::metric::HistogramSample;
use crate::nlog::{
    ChunkKind, SCHEMA_VERSION, file_preamble, put_chunk, put_str, put_varint, put_zigzag,
};
use crate::record::{FaultRecord, Record};
use crate::schema::{EventDescriptor, FieldKind, MetricDescriptor, SchemaKey};
use crate::session::SessionMetadata;

/// One metric's reading for a [`ChunkKind::MetricSnapshot`].
pub(crate) enum SnapshotValue {
    Counter { total: u64, delta: u64 },
    Gauge(u64),
    Histogram(HistogramSample),
}

/// Encodes chunks into a caller-owned buffer, reusing one payload scratch.
#[derive(Default)]
pub(crate) struct ChunkWriter {
    payload: Vec<u8>,
}

/// Preamble plus header chunk: the first bytes of every `.nlog` file.
/// `reason` distinguishes a session log (`"session"`) from a snapshot.
pub fn encode_header(meta: &SessionMetadata, reason: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    file_preamble(&mut out);
    let mut payload = Vec::with_capacity(256);
    put_varint(&mut payload, u64::from(SCHEMA_VERSION));
    put_varint(&mut payload, meta.session_id);
    put_str(&mut payload, reason);
    put_str(&mut payload, &meta.app_id);
    put_str(&mut payload, &meta.app_name);
    put_str(&mut payload, &meta.app_version);
    put_str(&mut payload, meta.build_id.as_deref().unwrap_or(""));
    put_str(&mut payload, &meta.framework_version);
    put_str(&mut payload, std::env::consts::OS);
    put_str(&mut payload, std::env::consts::ARCH);
    put_varint(&mut payload, u64::from(meta.pid));
    put_varint(&mut payload, meta.wall_start_unix_ns);
    put_varint(&mut payload, meta.monotonic_start_ns);
    put_varint(&mut payload, meta.extra.len() as u64);
    for (key, value) in &meta.extra {
        put_str(&mut payload, key);
        put_str(&mut payload, value);
    }
    put_chunk(&mut out, ChunkKind::Header, &payload);
    out
}

fn put_key(out: &mut Vec<u8>, key: SchemaKey) {
    put_varint(out, u64::from(key.domain.0));
    put_varint(out, u64::from(key.id));
}

fn put_values(out: &mut Vec<u8>, event: &EventDescriptor, record: &Record) {
    let len = usize::from(record.len);
    out.push(record.len);
    for (index, bits) in record.values[..len].iter().copied().enumerate() {
        let kind = event
            .fields
            .get(index)
            .map_or(FieldKind::U64, |field| field.kind);
        out.push(kind as u8);
        match kind {
            FieldKind::U64 => put_varint(out, bits),
            FieldKind::I64 => put_zigzag(out, bits as i64),
            FieldKind::F64 => out.extend_from_slice(&bits.to_le_bytes()),
            FieldKind::Bool => out.push(u8::from(bits != 0)),
        }
    }
}

impl ChunkWriter {
    fn finish(&mut self, out: &mut Vec<u8>, kind: ChunkKind) {
        put_chunk(out, kind, &self.payload);
        self.payload.clear();
    }

    pub(crate) fn event_schema(&mut self, out: &mut Vec<u8>, event: &EventDescriptor) {
        let p = &mut self.payload;
        put_key(p, event.key());
        put_str(p, event.name);
        p.push(event.severity as u8);
        put_varint(p, event.fields.len() as u64);
        for field in event.fields {
            put_str(p, field.name);
            p.push(field.kind as u8);
        }
        self.finish(out, ChunkKind::EventSchema);
    }

    pub(crate) fn metric_schema(&mut self, out: &mut Vec<u8>, metric: &MetricDescriptor) {
        let p = &mut self.payload;
        put_key(p, metric.key());
        put_str(p, metric.name);
        p.push(metric.kind as u8);
        put_str(p, metric.unit);
        self.finish(out, ChunkKind::MetricSchema);
    }

    pub(crate) fn thread_name(&mut self, out: &mut Vec<u8>, thread: u32, name: &str) {
        put_varint(&mut self.payload, u64::from(thread));
        put_str(&mut self.payload, name);
        self.finish(out, ChunkKind::ThreadName);
    }

    /// `records` must be sorted by timestamp.
    pub(crate) fn events(&mut self, out: &mut Vec<u8>, records: &[Record]) {
        if records.is_empty() {
            return;
        }
        let p = &mut self.payload;
        put_varint(p, records.len() as u64);
        let mut previous = 0u64;
        for record in records {
            put_varint(p, record.ts_ns.saturating_sub(previous));
            previous = previous.max(record.ts_ns);
            put_key(p, record.event.key());
            put_varint(p, u64::from(record.thread));
            put_values(p, record.event, record);
        }
        self.finish(out, ChunkKind::Events);
    }

    pub(crate) fn fault(&mut self, out: &mut Vec<u8>, fault: &FaultRecord) {
        let record = &fault.record;
        let p = &mut self.payload;
        put_varint(p, record.ts_ns);
        put_key(p, record.event.key());
        put_varint(p, u64::from(record.thread));
        put_values(p, record.event, record);
        match &fault.message {
            Some(message) => {
                p.push(1);
                put_str(p, message);
            }
            None => p.push(0),
        }
        self.finish(out, ChunkKind::Fault);
    }

    pub(crate) fn metric_snapshot(
        &mut self,
        out: &mut Vec<u8>,
        ts_ns: u64,
        items: &[(MetricDescriptor, SnapshotValue)],
    ) {
        let p = &mut self.payload;
        put_varint(p, ts_ns);
        put_varint(p, items.len() as u64);
        for (descriptor, value) in items {
            put_key(p, descriptor.key());
            p.push(descriptor.kind as u8);
            match value {
                SnapshotValue::Counter { total, delta } => {
                    put_varint(p, *total);
                    put_varint(p, *delta);
                }
                SnapshotValue::Gauge(value) => put_varint(p, *value),
                SnapshotValue::Histogram(sample) => {
                    put_varint(p, sample.count);
                    put_varint(p, sample.sum);
                    put_varint(p, sample.min);
                    put_varint(p, sample.max);
                    put_varint(p, sample.buckets.len() as u64);
                    for (bucket, count) in &sample.buckets {
                        p.push(*bucket);
                        put_varint(p, *count);
                    }
                }
            }
        }
        self.finish(out, ChunkKind::MetricSnapshot);
    }

    pub(crate) fn session_info(
        &mut self,
        out: &mut Vec<u8>,
        ts_ns: u64,
        pairs: &[(String, String)],
    ) {
        put_varint(&mut self.payload, ts_ns);
        put_varint(&mut self.payload, pairs.len() as u64);
        for (key, value) in pairs {
            put_str(&mut self.payload, key);
            put_str(&mut self.payload, value);
        }
        self.finish(out, ChunkKind::SessionInfo);
    }

    pub(crate) fn clock_sync(&mut self, out: &mut Vec<u8>, ts_ns: u64, wall_unix_ns: u64) {
        put_varint(&mut self.payload, ts_ns);
        put_varint(&mut self.payload, wall_unix_ns);
        self.finish(out, ChunkKind::ClockSync);
    }

    pub(crate) fn dropped(
        &mut self,
        out: &mut Vec<u8>,
        ts_ns: u64,
        threads: &[(u32, u64, u64)],
        stats: &[(&str, u64)],
    ) {
        let p = &mut self.payload;
        put_varint(p, ts_ns);
        put_varint(p, threads.len() as u64);
        for (thread, events, faults) in threads {
            put_varint(p, u64::from(*thread));
            put_varint(p, *events);
            put_varint(p, *faults);
        }
        put_varint(p, stats.len() as u64);
        for (name, value) in stats {
            put_str(p, name);
            put_varint(p, *value);
        }
        self.finish(out, ChunkKind::Dropped);
    }

    pub(crate) fn marker(&mut self, out: &mut Vec<u8>, ts_ns: u64, text: &str) {
        put_varint(&mut self.payload, ts_ns);
        put_str(&mut self.payload, text);
        self.finish(out, ChunkKind::Marker);
    }
}
