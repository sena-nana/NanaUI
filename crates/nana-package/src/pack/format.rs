//! `.nrpack` v1 byte layout. All integers little-endian.
//!
//! ```text
//! [header: 4096 B][data: entry extents, zero padding][TOC, 4096-aligned]
//! ```
//!
//! Header (fields in bytes 0..256, the rest zero):
//!
//! | off | size | field |
//! | --- | --- | --- |
//! | 0 | 8 | magic `NANAPACK` |
//! | 8 | 2 | format version (1) |
//! | 10 | 2 | flags: bit0 encrypted (blocks and TOC), bit1 signed |
//! | 12 | 4 | header length (4096) |
//! | 16 | 16 | pack id ([`crate::hash::pack_id`]) |
//! | 32 | 4 | block size (plaintext bytes per block) |
//! | 36 | 1 | resource class |
//! | 37 | 3 | reserved |
//! | 40 | 8 | content key id (zero when not encrypted) |
//! | 48 | 4 | key generation |
//! | 52 | 4 | entry count |
//! | 56 | 8 | TOC offset |
//! | 64 | 8 | TOC stored length |
//! | 72 | 8 | TOC plaintext length |
//! | 80 | 8 | end of data region |
//! | 88 | 32 | BLAKE3 of the stored TOC |
//! | 120 | 64 | reserved |
//! | 184 | 8 | publisher key id (zero when unsigned) |
//! | 192 | 64 | Ed25519 signature over `SIGNATURE_CONTEXT ‖ header[0..192]` |
//!
//! TOC plaintext: a 32-byte TOC header, `entry_count` 96-byte entries sorted
//! by key bytes, `block_count` 32-byte block records, then the UTF-8 string
//! table. Encrypted packs store `nonce(24) ‖ ciphertext ‖ tag(16)` instead.
//!
//! The signature covers the TOC hash, the TOC covers each block record's
//! hash, and each entry's plaintext hash, so one signature check at mount
//! authenticates everything read later.

pub const MAGIC: &[u8; 8] = b"NANAPACK";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 4096;
/// Offset of the entry count in the header (see the table above).
pub const ENTRY_COUNT_OFFSET: usize = 52;
/// Bytes of the header covered by the signature.
pub const SIGNED_LEN: usize = 192;
pub const SIGNATURE_CONTEXT: &[u8] = b"nana.nrpack.v1.header\0";

pub const FLAG_ENCRYPTED: u16 = 1 << 0;
pub const FLAG_SIGNED: u16 = 1 << 1;
pub const KNOWN_FLAGS: u16 = FLAG_ENCRYPTED | FLAG_SIGNED;

pub const TOC_MAGIC: &[u8; 4] = b"NTOC";
pub const TOC_VERSION: u16 = 1;
pub const TOC_HEADER_LEN: usize = 32;
pub const ENTRY_LEN: usize = 96;
pub const BLOCK_LEN: usize = 32;
/// Alignment of the TOC and of the first extent.
pub const PAGE: u64 = 4096;
/// Alignment of each entry extent.
pub const EXTENT_ALIGN: u64 = 16;

pub const DEFAULT_BLOCK_SIZE: u32 = 64 * 1024;
pub const MIN_BLOCK_SIZE: u32 = 4 * 1024;
pub const MAX_BLOCK_SIZE: u32 = 4 * 1024 * 1024;
/// Upper bound on a stored TOC, checked before it is read.
pub const MAX_TOC_LEN: u64 = 256 * 1024 * 1024;
pub const MAX_KEY_LEN: usize = 1024;

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
/// Bytes a sealed record adds to its payload.
pub const SEAL_OVERHEAD: usize = NONCE_LEN + TAG_LEN;

/// Block payload is a zstd frame (otherwise stored).
pub const BLOCK_FLAG_COMPRESSED: u8 = 1 << 0;
/// Requested codec of an entry (informational; the per-block flag decides).
pub const CODEC_STORED: u8 = 0;
pub const CODEC_ZSTD: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: u16,
    pub flags: u16,
    pub pack_id: [u8; 16],
    pub block_size: u32,
    pub class: u8,
    pub key_id: [u8; 8],
    pub key_generation: u32,
    pub entry_count: u32,
    pub toc_offset: u64,
    pub toc_stored_len: u64,
    pub toc_plain_len: u64,
    pub data_end: u64,
    pub toc_hash: [u8; 32],
    pub publisher_key_id: [u8; 8],
    pub signature: [u8; 64],
}

impl Header {
    pub fn encrypted(&self) -> bool {
        self.flags & FLAG_ENCRYPTED != 0
    }

    pub fn signed(&self) -> bool {
        self.flags & FLAG_SIGNED != 0
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0u8; HEADER_LEN];
        out[0..8].copy_from_slice(MAGIC);
        put_u16(&mut out, 8, self.version);
        put_u16(&mut out, 10, self.flags);
        put_u32(&mut out, 12, HEADER_LEN as u32);
        out[16..32].copy_from_slice(&self.pack_id);
        put_u32(&mut out, 32, self.block_size);
        out[36] = self.class;
        out[40..48].copy_from_slice(&self.key_id);
        put_u32(&mut out, 48, self.key_generation);
        put_u32(&mut out, ENTRY_COUNT_OFFSET, self.entry_count);
        put_u64(&mut out, 56, self.toc_offset);
        put_u64(&mut out, 64, self.toc_stored_len);
        put_u64(&mut out, 72, self.toc_plain_len);
        put_u64(&mut out, 80, self.data_end);
        out[88..120].copy_from_slice(&self.toc_hash);
        out[184..192].copy_from_slice(&self.publisher_key_id);
        out[192..256].copy_from_slice(&self.signature);
        out
    }

    /// Parse the fixed fields. Semantic checks (flags, sizes, offsets) are
    /// the reader's.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN || &bytes[0..8] != MAGIC {
            return None;
        }
        if get_u32(bytes, 12) as usize != HEADER_LEN {
            return None;
        }
        Some(Self {
            version: get_u16(bytes, 8),
            flags: get_u16(bytes, 10),
            pack_id: array(&bytes[16..32]),
            block_size: get_u32(bytes, 32),
            class: bytes[36],
            key_id: array(&bytes[40..48]),
            key_generation: get_u32(bytes, 48),
            entry_count: get_u32(bytes, ENTRY_COUNT_OFFSET),
            toc_offset: get_u64(bytes, 56),
            toc_stored_len: get_u64(bytes, 64),
            toc_plain_len: get_u64(bytes, 72),
            data_end: get_u64(bytes, 80),
            toc_hash: array(&bytes[88..120]),
            publisher_key_id: array(&bytes[184..192]),
            signature: array(&bytes[192..256]),
        })
    }

    /// The message the publisher signs: context ‖ encoded header up to the
    /// signature field. Reserved bytes are included, so they must be zero.
    pub fn signing_message(encoded: &[u8]) -> Vec<u8> {
        let mut message = SIGNATURE_CONTEXT.to_vec();
        message.extend_from_slice(&encoded[..SIGNED_LEN]);
        message
    }

    /// Reserved header bytes that must be zero.
    pub fn reserved_is_zero(encoded: &[u8]) -> bool {
        encoded[37..40].iter().all(|&b| b == 0)
            && encoded[120..184].iter().all(|&b| b == 0)
            && encoded[256..HEADER_LEN].iter().all(|&b| b == 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TocHeader {
    pub entry_count: u32,
    pub string_table_len: u32,
    pub block_count: u32,
}

impl TocHeader {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.resize(start + TOC_HEADER_LEN, 0);
        let b = &mut out[start..];
        b[0..4].copy_from_slice(TOC_MAGIC);
        put_u16(b, 4, TOC_VERSION);
        put_u32(b, 8, self.entry_count);
        put_u32(b, 12, self.string_table_len);
        put_u32(b, 16, self.block_count);
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < TOC_HEADER_LEN
            || &bytes[0..4] != TOC_MAGIC
            || get_u16(bytes, 4) != TOC_VERSION
        {
            return None;
        }
        Some(Self {
            entry_count: get_u32(bytes, 8),
            string_table_len: get_u32(bytes, 12),
            block_count: get_u32(bytes, 16),
        })
    }

    /// Total plaintext TOC length this header implies.
    pub fn plain_len(&self) -> u64 {
        TOC_HEADER_LEN as u64
            + self.entry_count as u64 * ENTRY_LEN as u64
            + self.block_count as u64 * BLOCK_LEN as u64
            + self.string_table_len as u64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocEntry {
    pub key_offset: u32,
    pub key_len: u32,
    pub codec: u8,
    pub codec_level: i32,
    pub plain_len: u64,
    pub plain_hash: [u8; 32],
    pub extent_offset: u64,
    pub extent_capacity: u64,
    pub first_block: u32,
    pub block_count: u32,
}

/// Byte 9 of an entry is reserved (zero): whether blocks are sealed is the
/// pack header's `FLAG_ENCRYPTED`, one fact in one place.
impl TocEntry {
    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.resize(start + ENTRY_LEN, 0);
        let b = &mut out[start..];
        put_u32(b, 0, self.key_offset);
        put_u32(b, 4, self.key_len);
        b[8] = self.codec;
        put_u32(b, 12, self.codec_level as u32);
        put_u64(b, 16, self.plain_len);
        b[24..56].copy_from_slice(&self.plain_hash);
        put_u64(b, 56, self.extent_offset);
        put_u64(b, 64, self.extent_capacity);
        put_u32(b, 72, self.first_block);
        put_u32(b, 76, self.block_count);
    }

    pub fn decode(b: &[u8]) -> Self {
        Self {
            key_offset: get_u32(b, 0),
            key_len: get_u32(b, 4),
            codec: b[8],
            codec_level: get_u32(b, 12) as i32,
            plain_len: get_u64(b, 16),
            plain_hash: array(&b[24..56]),
            extent_offset: get_u64(b, 56),
            extent_capacity: get_u64(b, 64),
            first_block: get_u32(b, 72),
            block_count: get_u32(b, 76),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRecord {
    /// Bytes on disk (payload, plus nonce and tag when sealed).
    pub stored_len: u32,
    /// Plaintext bytes after decompression.
    pub plain_len: u32,
    pub flags: u8,
    /// First 16 bytes of BLAKE3 over the stored record.
    pub record_hash: [u8; 16],
}

impl BlockRecord {
    pub fn compressed(&self) -> bool {
        self.flags & BLOCK_FLAG_COMPRESSED != 0
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.resize(start + BLOCK_LEN, 0);
        let b = &mut out[start..];
        put_u32(b, 0, self.stored_len);
        put_u32(b, 4, self.plain_len);
        b[8] = self.flags;
        b[16..32].copy_from_slice(&self.record_hash);
    }

    pub fn decode(b: &[u8]) -> Self {
        Self {
            stored_len: get_u32(b, 0),
            plain_len: get_u32(b, 4),
            flags: b[8],
            record_hash: array(&b[16..32]),
        }
    }
}

/// Hash a stored block record is checked against before anything else.
pub fn record_hash(record: &[u8]) -> [u8; 16] {
    let full = blake3::hash(record);
    array(&full.as_bytes()[..16])
}

const BLOCK_AAD_CONTEXT: &[u8; 12] = b"nrpack1.blk\0";
pub const BLOCK_AAD_LEN: usize = 12 + 16 + 16 + 4 + 4 + 4 + 1;

/// Associated data of a sealed block. Binds the block to its pack, entry
/// (`entry_key_id` = [`crate::hash::entry_key_id`] of its key, computed once
/// per entry), position, key generation, plaintext length and compression
/// flag, so a block moved or relabelled fails authentication.
pub fn block_aad(
    pack_id: &[u8; 16],
    entry_key_id: &[u8; 16],
    block_index: u32,
    key_generation: u32,
    plain_len: u32,
    flags: u8,
) -> [u8; BLOCK_AAD_LEN] {
    let mut aad = [0u8; BLOCK_AAD_LEN];
    aad[..12].copy_from_slice(BLOCK_AAD_CONTEXT);
    aad[12..28].copy_from_slice(pack_id);
    aad[28..44].copy_from_slice(entry_key_id);
    aad[44..48].copy_from_slice(&block_index.to_le_bytes());
    aad[48..52].copy_from_slice(&key_generation.to_le_bytes());
    aad[52..56].copy_from_slice(&plain_len.to_le_bytes());
    aad[56] = flags;
    aad
}

/// Associated data of a sealed TOC: every header field before the TOC hash
/// (id, flags, class, block size, key, counts, offsets and lengths). All are
/// known before sealing, since a sealed TOC is always `SEAL_OVERHEAD` longer
/// than its plaintext. An encrypted pack whose header was edited, even
/// without a signature, fails TOC authentication.
pub fn toc_aad(encoded_header: &[u8]) -> Vec<u8> {
    let mut aad = b"nrpack1.toc\0".to_vec();
    aad.extend_from_slice(&encoded_header[..TOC_AAD_HEADER_LEN]);
    aad
}

/// Header bytes bound into the TOC AAD (everything before the TOC hash).
pub const TOC_AAD_HEADER_LEN: usize = 88;

pub fn align_up(value: u64, align: u64) -> u64 {
    value.div_ceil(align) * align
}

fn array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes[..N]);
    out
}

fn put_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_u64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
pub(crate) fn get_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(array(&b[at..at + 2]))
}
pub(crate) fn get_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(array(&b[at..at + 4]))
}
pub(crate) fn get_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(array(&b[at..at + 8]))
}
