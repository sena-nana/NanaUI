//! `.nlog` decoder. Tolerates a truncated or corrupted tail: decoding stops
//! at the first bad chunk and [`NlogFile::truncated`] says so.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use crate::metric::HistogramSample;
use crate::nlog::{CHUNK_OVERHEAD, ChunkKind, FORMAT_VERSION, MAGIC, crc::crc32};
use crate::schema::{Domain, FieldKind, MetricKind, SchemaKey, Severity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    NotNlog,
    UnsupportedVersion(u16),
    MissingHeader,
    Io(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotNlog => f.write_str("not an .nlog file"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported .nlog format version {v}"),
            Self::MissingHeader => f.write_str(".nlog file has no readable header"),
            Self::Io(e) => write!(f, "read failed: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionHeader {
    pub schema_version: u32,
    pub session_id: u64,
    /// `"session"` for a running log, `"snapshot:<reason>"` for a snapshot.
    pub reason: String,
    pub app_id: String,
    pub app_name: String,
    pub app_version: String,
    pub build_id: String,
    pub framework_version: String,
    pub os: String,
    pub arch: String,
    pub pid: u32,
    pub wall_start_unix_ns: u64,
    pub extra: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventSchema {
    pub key: SchemaKey,
    pub name: String,
    pub severity: Severity,
    pub fields: Vec<(String, FieldKind)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricSchema {
    pub key: SchemaKey,
    pub name: String,
    pub kind: MetricKind,
    pub unit: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    U64(u64),
    I64(i64),
    F64(f64),
    Bool(bool),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::U64(v) => write!(f, "{v}"),
            Self::I64(v) => write!(f, "{v}"),
            Self::F64(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetricSample {
    Counter {
        key: SchemaKey,
        total: u64,
        delta: u64,
    },
    Gauge {
        key: SchemaKey,
        value: u64,
    },
    Histogram {
        key: SchemaKey,
        sample: HistogramSample,
    },
}

impl MetricSample {
    pub fn key(&self) -> SchemaKey {
        match self {
            Self::Counter { key, .. } | Self::Gauge { key, .. } | Self::Histogram { key, .. } => {
                *key
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Event {
        ts_ns: u64,
        key: SchemaKey,
        thread: u32,
        values: Vec<Value>,
    },
    Fault {
        ts_ns: u64,
        key: SchemaKey,
        thread: u32,
        values: Vec<Value>,
        message: Option<String>,
    },
    Metrics {
        ts_ns: u64,
        samples: Vec<MetricSample>,
    },
    SessionInfo {
        ts_ns: u64,
        pairs: Vec<(String, String)>,
    },
    ClockSync {
        ts_ns: u64,
        wall_unix_ns: u64,
    },
    Dropped {
        ts_ns: u64,
        /// `(thread, events dropped, faults dropped)`, cumulative.
        threads: Vec<(u32, u64, u64)>,
        stats: Vec<(String, u64)>,
    },
    Marker {
        ts_ns: u64,
        text: String,
    },
}

impl Entry {
    pub fn ts_ns(&self) -> u64 {
        match self {
            Self::Event { ts_ns, .. }
            | Self::Fault { ts_ns, .. }
            | Self::Metrics { ts_ns, .. }
            | Self::SessionInfo { ts_ns, .. }
            | Self::ClockSync { ts_ns, .. }
            | Self::Dropped { ts_ns, .. }
            | Self::Marker { ts_ns, .. } => *ts_ns,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NlogFile {
    pub header: SessionHeader,
    pub event_schemas: BTreeMap<SchemaKey, EventSchema>,
    pub metric_schemas: BTreeMap<SchemaKey, MetricSchema>,
    pub threads: BTreeMap<u32, String>,
    /// In file order.
    pub entries: Vec<Entry>,
    /// The file ended mid-chunk or a chunk failed its checksum; `entries`
    /// holds everything before that point.
    pub truncated: bool,
    /// Chunks of a kind this reader does not know (a newer writer); skipped.
    pub unknown_chunks: usize,
}

pub fn read_file(path: impl AsRef<Path>) -> Result<NlogFile, DecodeError> {
    let bytes = std::fs::read(path).map_err(|e| DecodeError::Io(e.to_string()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<NlogFile, DecodeError> {
    if bytes.len() < MAGIC.len() + 2 || bytes[..MAGIC.len()] != MAGIC {
        return Err(DecodeError::NotNlog);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != FORMAT_VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let mut file = NlogFile::default();
    let mut rest = &bytes[MAGIC.len() + 2..];
    let mut saw_header = false;
    while !rest.is_empty() {
        let Some((kind, payload, next)) = split_chunk(rest) else {
            file.truncated = true;
            break;
        };
        rest = next;
        let Some(kind) = ChunkKind::from_u8(kind) else {
            file.unknown_chunks += 1;
            continue;
        };
        if !saw_header && kind != ChunkKind::Header {
            return Err(DecodeError::MissingHeader);
        }
        let mut cursor = Cursor { bytes: payload };
        if decode_chunk(&mut file, kind, &mut cursor).is_none() {
            // A chunk that passed its CRC but does not parse was written by
            // something else; treat it like corruption.
            file.truncated = true;
            break;
        }
        saw_header = true;
    }
    if !saw_header {
        return Err(DecodeError::MissingHeader);
    }
    Ok(file)
}

fn split_chunk(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    if bytes.len() < CHUNK_OVERHEAD {
        return None;
    }
    let len = u32::from_le_bytes(bytes[1..5].try_into().ok()?) as usize;
    let end = 5usize.checked_add(len)?;
    let crc_end = end.checked_add(4)?;
    if bytes.len() < crc_end {
        return None;
    }
    let stored = u32::from_le_bytes(bytes[end..crc_end].try_into().ok()?);
    if crc32(&bytes[..end]) != stored {
        return None;
    }
    Some((bytes[0], &bytes[5..end], &bytes[crc_end..]))
}

struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Option<u8> {
        let (&first, rest) = self.bytes.split_first()?;
        self.bytes = rest;
        Some(first)
    }

    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            // The tenth byte may only carry the top bit.
            if shift == 63 && byte > 1 {
                return None;
            }
            value |= u64::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn u32(&mut self) -> Option<u32> {
        u32::try_from(self.varint()?).ok()
    }

    fn zigzag(&mut self) -> Option<i64> {
        let raw = self.varint()?;
        Some(((raw >> 1) as i64) ^ -((raw & 1) as i64))
    }

    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        if self.bytes.len() < len {
            return None;
        }
        let (head, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Some(head)
    }

    fn string(&mut self) -> Option<String> {
        let len = usize::try_from(self.varint()?).ok()?;
        String::from_utf8(self.bytes(len)?.to_vec()).ok()
    }

    fn key(&mut self) -> Option<SchemaKey> {
        Some(SchemaKey {
            domain: Domain(u16::try_from(self.varint()?).ok()?),
            id: self.u32()?,
        })
    }

    fn values(&mut self) -> Option<Vec<Value>> {
        let count = self.u8()?;
        (0..count)
            .map(|_| {
                Some(match FieldKind::from_u8(self.u8()?)? {
                    FieldKind::U64 => Value::U64(self.varint()?),
                    FieldKind::I64 => Value::I64(self.zigzag()?),
                    FieldKind::F64 => {
                        Value::F64(f64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
                    }
                    FieldKind::Bool => Value::Bool(self.u8()? != 0),
                })
            })
            .collect()
    }

    /// Bound a declared element count by the bytes left, so a corrupt count
    /// cannot ask for a giant allocation.
    fn count(&mut self) -> Option<usize> {
        let count = usize::try_from(self.varint()?).ok()?;
        (count <= self.bytes.len()).then_some(count)
    }
}

fn decode_chunk(file: &mut NlogFile, kind: ChunkKind, c: &mut Cursor<'_>) -> Option<()> {
    match kind {
        ChunkKind::Header => {
            let h = &mut file.header;
            h.schema_version = c.u32()?;
            h.session_id = c.varint()?;
            h.reason = c.string()?;
            h.app_id = c.string()?;
            h.app_name = c.string()?;
            h.app_version = c.string()?;
            h.build_id = c.string()?;
            h.framework_version = c.string()?;
            h.os = c.string()?;
            h.arch = c.string()?;
            h.pid = c.u32()?;
            h.wall_start_unix_ns = c.varint()?;
            let extra = c.count()?;
            h.extra = (0..extra)
                .map(|_| Some((c.string()?, c.string()?)))
                .collect::<Option<_>>()?;
        }
        ChunkKind::EventSchema => {
            let key = c.key()?;
            let name = c.string()?;
            let severity = Severity::from_u8(c.u8()?)?;
            let count = c.count()?;
            let fields = (0..count)
                .map(|_| Some((c.string()?, FieldKind::from_u8(c.u8()?)?)))
                .collect::<Option<_>>()?;
            file.event_schemas.insert(
                key,
                EventSchema {
                    key,
                    name,
                    severity,
                    fields,
                },
            );
        }
        ChunkKind::MetricSchema => {
            let key = c.key()?;
            let name = c.string()?;
            let kind = MetricKind::from_u8(c.u8()?)?;
            let unit = c.string()?;
            file.metric_schemas.insert(
                key,
                MetricSchema {
                    key,
                    name,
                    kind,
                    unit,
                },
            );
        }
        ChunkKind::ThreadName => {
            let thread = c.u32()?;
            let name = c.string()?;
            file.threads.insert(thread, name);
        }
        ChunkKind::Events => {
            let count = c.count()?;
            let mut ts = 0u64;
            for _ in 0..count {
                ts = ts.checked_add(c.varint()?)?;
                let key = c.key()?;
                let thread = c.u32()?;
                let values = c.values()?;
                file.entries.push(Entry::Event {
                    ts_ns: ts,
                    key,
                    thread,
                    values,
                });
            }
        }
        ChunkKind::Fault => {
            let ts_ns = c.varint()?;
            let key = c.key()?;
            let thread = c.u32()?;
            let values = c.values()?;
            let message = match c.u8()? {
                0 => None,
                _ => Some(c.string()?),
            };
            file.entries.push(Entry::Fault {
                ts_ns,
                key,
                thread,
                values,
                message,
            });
        }
        ChunkKind::MetricSnapshot => {
            let ts_ns = c.varint()?;
            let count = c.count()?;
            let mut samples = Vec::new();
            for _ in 0..count {
                let key = c.key()?;
                samples.push(match MetricKind::from_u8(c.u8()?)? {
                    MetricKind::Counter => MetricSample::Counter {
                        key,
                        total: c.varint()?,
                        delta: c.varint()?,
                    },
                    MetricKind::Gauge => MetricSample::Gauge {
                        key,
                        value: c.varint()?,
                    },
                    MetricKind::Histogram => {
                        let count = c.varint()?;
                        let sum = c.varint()?;
                        let min = c.varint()?;
                        let max = c.varint()?;
                        let buckets = c.count()?;
                        let buckets = (0..buckets)
                            .map(|_| {
                                let bucket = c.u8()?;
                                (usize::from(bucket) < crate::metric::HISTOGRAM_BUCKETS)
                                    .then_some(())?;
                                Some((bucket, c.varint()?))
                            })
                            .collect::<Option<_>>()?;
                        MetricSample::Histogram {
                            key,
                            sample: HistogramSample {
                                count,
                                sum,
                                min,
                                max,
                                buckets,
                            },
                        }
                    }
                });
            }
            file.entries.push(Entry::Metrics { ts_ns, samples });
        }
        ChunkKind::SessionInfo => {
            let ts_ns = c.varint()?;
            let count = c.count()?;
            let pairs = (0..count)
                .map(|_| Some((c.string()?, c.string()?)))
                .collect::<Option<_>>()?;
            file.entries.push(Entry::SessionInfo { ts_ns, pairs });
        }
        ChunkKind::ClockSync => {
            let ts_ns = c.varint()?;
            let wall_unix_ns = c.varint()?;
            file.entries.push(Entry::ClockSync {
                ts_ns,
                wall_unix_ns,
            });
        }
        ChunkKind::Dropped => {
            let ts_ns = c.varint()?;
            let count = c.count()?;
            let threads = (0..count)
                .map(|_| Some((c.u32()?, c.varint()?, c.varint()?)))
                .collect::<Option<_>>()?;
            let count = c.count()?;
            let stats = (0..count)
                .map(|_| Some((c.string()?, c.varint()?)))
                .collect::<Option<_>>()?;
            file.entries.push(Entry::Dropped {
                ts_ns,
                threads,
                stats,
            });
        }
        ChunkKind::Marker => {
            let ts_ns = c.varint()?;
            let text = c.string()?;
            file.entries.push(Entry::Marker { ts_ns, text });
        }
    }
    Some(())
}

impl NlogFile {
    pub fn event_name(&self, key: SchemaKey) -> Option<&str> {
        self.event_schemas.get(&key).map(|s| s.name.as_str())
    }

    pub fn metric_name(&self, key: SchemaKey) -> Option<&str> {
        self.metric_schemas.get(&key).map(|s| s.name.as_str())
    }

    /// Latest `SessionInfo` value for `key`.
    pub fn session_info(&self, key: &str) -> Option<&str> {
        self.entries.iter().rev().find_map(|entry| match entry {
            Entry::SessionInfo { pairs, .. } => pairs
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str()),
            _ => None,
        })
    }
}
