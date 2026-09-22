//! `.nlog`: the persisted diagnostics format (schema v1).
//!
//! ```text
//! file   := MAGIC (8 bytes) FORMAT_VERSION (u16 LE) chunk*
//! chunk  := kind (u8) len (u32 LE) payload[len] crc32 (u32 LE over kind+len+payload)
//! ```
//!
//! The first chunk is always [`ChunkKind::Header`]. Every event or metric is
//! preceded, somewhere earlier in the same file, by its schema chunk, so a
//! reader needs nothing but the file. A crash can cut the last chunk short;
//! the reader stops at the first chunk that is truncated or fails its CRC and
//! reports everything before it.
//!
//! Payload integers are LEB128 varints, signed ones zigzag-encoded, `f64` is
//! 8 bytes LE, strings are `varint len` + UTF-8.

mod crc;
mod reader;
pub(crate) mod writer;

pub use reader::{
    DecodeError, Entry, EventSchema, MetricSample, MetricSchema, NlogFile, SessionHeader, Value,
    decode, read_file,
};
pub(crate) use writer::ChunkWriter;
pub use writer::encode_header;

pub const MAGIC: [u8; 8] = *b"NANALOG\0";
pub const FORMAT_VERSION: u16 = 1;
/// Version of the chunk payload layouts below.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ChunkKind {
    Header = 1,
    EventSchema = 2,
    MetricSchema = 3,
    ThreadName = 4,
    Events = 5,
    Fault = 6,
    MetricSnapshot = 7,
    SessionInfo = 8,
    ClockSync = 9,
    Dropped = 10,
    Marker = 11,
}

impl ChunkKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Header,
            2 => Self::EventSchema,
            3 => Self::MetricSchema,
            4 => Self::ThreadName,
            5 => Self::Events,
            6 => Self::Fault,
            7 => Self::MetricSnapshot,
            8 => Self::SessionInfo,
            9 => Self::ClockSync,
            10 => Self::Dropped,
            11 => Self::Marker,
            _ => return None,
        })
    }
}

/// Bytes of framing around each payload.
pub(crate) const CHUNK_OVERHEAD: usize = 1 + 4 + 4;

pub(crate) fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub(crate) fn put_zigzag(out: &mut Vec<u8>, value: i64) {
    put_varint(out, ((value << 1) ^ (value >> 63)) as u64);
}

pub(crate) fn put_str(out: &mut Vec<u8>, value: &str) {
    put_varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

/// Frame `payload` as one chunk onto `out`.
pub(crate) fn put_chunk(out: &mut Vec<u8>, kind: ChunkKind, payload: &[u8]) {
    let start = out.len();
    out.push(kind as u8);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    let crc = crc::crc32(&out[start..]);
    out.extend_from_slice(&crc.to_le_bytes());
}

pub(crate) fn file_preamble(out: &mut Vec<u8>) {
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
}
