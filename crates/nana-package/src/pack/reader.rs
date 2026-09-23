use std::cmp::Ordering;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::time::Instant;

use super::format::{
    self, BLOCK_LEN, BlockRecord, ENTRY_LEN, FORMAT_VERSION, HEADER_LEN, Header, KNOWN_FLAGS,
    MAX_BLOCK_SIZE, MAX_TOC_LEN, MIN_BLOCK_SIZE, PAGE, SEAL_OVERHEAD, TOC_HEADER_LEN, TocEntry,
    TocHeader,
};
use super::{PackError, ResourceClass, seal};
use crate::keys::{ContentKey, KeyId, KeyProvider, KeyRequest, TrustPolicy};

/// What the package manifest says a pack must be. Checked at mount, so an
/// older (validly signed) pack cannot be swapped in for the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedPack {
    pub pack_id: [u8; 16],
    pub toc_hash: [u8; 32],
    pub class: ResourceClass,
    pub encrypted: bool,
    pub signed: bool,
    pub key_generation: u32,
}

/// Metadata of one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    pub plain_len: u64,
    pub block_count: u32,
}

/// Work done by reads, for the caller's diagnostics. This crate records
/// nothing itself.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReadStats {
    pub bytes_read: u64,
    pub auth_ns: u64,
    pub decompress_ns: u64,
}

/// A mounted pack. Opening verifies the header, the publisher signature (as
/// the [`TrustPolicy`] demands), the TOC hash and, for encrypted packs, the
/// TOC's authentication. Reads are positional, so one reader serves any
/// number of threads.
pub struct PackReader {
    file: File,
    name: String,
    header: Header,
    class: ResourceClass,
    toc: Vec<u8>,
    toc_header: TocHeader,
    key: Option<ContentKey>,
}

/// Hand-written: the TOC of an encrypted pack (entry names, hashes) and its
/// key must not end up in logs.
impl std::fmt::Debug for PackReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackReader")
            .field("name", &self.name)
            .field("class", &self.class)
            .field("entries", &self.toc_header.entry_count)
            .field("encrypted", &self.header.encrypted())
            .field("signed", &self.header.signed())
            .finish_non_exhaustive()
    }
}

impl PackReader {
    /// Open and verify `path`. `name` is the pack's manifest name, passed to
    /// the key provider. `expected` pins the pack to the manifest.
    pub fn open(
        path: &Path,
        name: &str,
        keys: &dyn KeyProvider,
        trust: &TrustPolicy,
        expected: Option<&ExpectedPack>,
    ) -> Result<Self, PackError> {
        let file = File::open(path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => PackError::Missing,
            _ => PackError::Io(error.to_string()),
        })?;
        let file_len = file.metadata().map_err(io_error)?.len();
        if file_len < HEADER_LEN as u64 {
            return Err(PackError::NotAPack);
        }
        let mut raw_header = vec![0u8; HEADER_LEN];
        read_exact_at(&file, &mut raw_header, 0)?;
        let header = Header::decode(&raw_header).ok_or(PackError::NotAPack)?;
        if header.version != FORMAT_VERSION {
            return Err(PackError::UnsupportedVersion(header.version));
        }
        let class = check_header(&header, &raw_header)?;

        match trust {
            TrustPolicy::AllowUnsigned => {}
            TrustPolicy::RequirePublisher(publisher) => {
                if !header.signed() {
                    return Err(PackError::SignatureRequired);
                }
                if header.publisher_key_id != publisher.id().0 {
                    return Err(PackError::UnknownPublisher);
                }
                if !publisher.verify(&Header::signing_message(&raw_header), &header.signature) {
                    return Err(PackError::SignatureInvalid);
                }
            }
        }
        if let Some(expected) = expected
            && (expected.pack_id != header.pack_id
                || expected.toc_hash != header.toc_hash
                || expected.class != class
                || expected.encrypted != header.encrypted()
                || expected.signed != header.signed()
                || expected.key_generation != header.key_generation)
        {
            return Err(PackError::UnexpectedPack);
        }

        if header.data_end < HEADER_LEN as u64
            || header.toc_offset < header.data_end
            || header.toc_offset % PAGE != 0
            || header.toc_stored_len > MAX_TOC_LEN
            || header.toc_offset.checked_add(header.toc_stored_len) != Some(file_len)
        {
            return Err(
                if file_len < header.toc_offset.saturating_add(header.toc_stored_len) {
                    PackError::Truncated
                } else {
                    PackError::BadHeader("TOC bounds")
                },
            );
        }
        let mut stored_toc = vec![0u8; header.toc_stored_len as usize];
        read_exact_at(&file, &mut stored_toc, header.toc_offset)?;
        if crate::hash::content(&stored_toc) != header.toc_hash {
            return Err(PackError::TocHashMismatch);
        }

        let key = if header.encrypted() {
            let request = KeyRequest {
                pack: name,
                key_id: KeyId(header.key_id),
                generation: header.key_generation,
                class,
            };
            Some(keys.content_key(&request).map_err(|_| {
                PackError::KeyUnavailable(format!(
                    "key {} generation {}",
                    KeyId(header.key_id),
                    header.key_generation
                ))
            })?)
        } else {
            None
        };
        let toc = match &key {
            Some(key) => {
                let aad = format::toc_aad(&raw_header);
                seal::open(key, &aad, &stored_toc).ok_or(PackError::TocAuthFailed)?
            }
            None => stored_toc,
        };
        if toc.len() as u64 != header.toc_plain_len {
            return Err(PackError::TocCorrupt("plaintext length"));
        }
        let toc_header = TocHeader::decode(&toc).ok_or(PackError::TocCorrupt("TOC header"))?;
        if toc_header.entry_count != header.entry_count
            || toc_header.plain_len() != toc.len() as u64
        {
            return Err(PackError::TocCorrupt("TOC sizes"));
        }

        Ok(Self {
            file,
            name: name.to_owned(),
            header,
            class,
            toc,
            toc_header,
            key,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn class(&self) -> ResourceClass {
        self.class
    }

    pub fn entry_count(&self) -> u32 {
        self.toc_header.entry_count
    }

    /// Plaintext TOC length, for diagnostics.
    pub fn toc_len(&self) -> u64 {
        self.toc.len() as u64
    }

    /// Every entry key in order. O(entries); for tools, not for startup.
    pub fn keys(&self) -> Result<Vec<String>, PackError> {
        (0..self.toc_header.entry_count)
            .map(|index| {
                let entry = self.entry_at(index);
                std::str::from_utf8(self.key_bytes(&entry)?)
                    .map(str::to_owned)
                    .map_err(|_| PackError::TocCorrupt("key is not UTF-8"))
            })
            .collect()
    }

    pub fn entry(&self, key: &str) -> Result<EntryInfo, PackError> {
        let entry = self.find(key)?;
        Ok(EntryInfo {
            plain_len: entry.plain_len,
            block_count: entry.block_count,
        })
    }

    /// Raw TOC entry and block records, for the packager's layout reuse.
    pub fn stored_entry(&self, key: &str) -> Result<(TocEntry, Vec<BlockRecord>), PackError> {
        let entry = self.find(key)?;
        let blocks = self.blocks(&entry)?;
        Ok((entry, blocks))
    }

    /// Plaintext TOC of the pack (after authentication), so a rebuild with
    /// an identical TOC can reuse the stored bytes.
    pub fn toc_plaintext(&self) -> &[u8] {
        &self.toc
    }

    /// The underlying file, for positional reads of stored bytes.
    pub fn read_stored(&self, offset: u64, len: u64) -> Result<Vec<u8>, PackError> {
        let mut bytes = vec![0u8; usize::try_from(len).map_err(|_| PackError::TooLarge { len })?];
        read_exact_at(&self.file, &mut bytes, offset)?;
        Ok(bytes)
    }

    /// Read and fully verify one entry. `max_bytes` is checked against the
    /// TOC before any data is read.
    pub fn read(
        &self,
        key: &str,
        max_bytes: u64,
        stats: &mut ReadStats,
    ) -> Result<Vec<u8>, PackError> {
        let entry = self.find(key)?;
        if entry.plain_len > max_bytes {
            return Err(PackError::TooLarge {
                len: entry.plain_len,
            });
        }
        let (entry, blocks, records) = self.records_of(entry, stats)?;
        let context = EntryContext {
            pack_id: &self.header.pack_id,
            block_size: self.header.block_size,
            key_generation: self.header.key_generation,
            key: self.key.as_ref(),
        };
        decode_records(
            &context,
            self.key_bytes(&entry)?,
            &blocks,
            &records,
            &entry.plain_hash,
            stats,
        )
    }

    /// Raw TOC entry, block records and stored record bytes of `key`, for
    /// the packager's reuse of unchanged entries. Nothing is verified here;
    /// pass the result through [`decode_records`] before trusting it.
    pub fn stored_records(
        &self,
        key: &str,
    ) -> Result<(TocEntry, Vec<BlockRecord>, Vec<u8>), PackError> {
        let entry = self.find(key)?;
        self.records_of(entry, &mut ReadStats::default())
    }

    fn records_of(
        &self,
        entry: TocEntry,
        stats: &mut ReadStats,
    ) -> Result<(TocEntry, Vec<BlockRecord>, Vec<u8>), PackError> {
        let blocks = self.blocks(&entry)?;
        let stored: u64 = blocks.iter().map(|block| block.stored_len as u64).sum();
        let extent_end = entry
            .extent_offset
            .checked_add(entry.extent_capacity)
            .filter(|&end| end <= self.header.data_end && entry.extent_offset >= HEADER_LEN as u64)
            .ok_or(PackError::TocCorrupt("extent bounds"))?;
        if stored > entry.extent_capacity || entry.extent_offset + stored > extent_end {
            return Err(PackError::TocCorrupt("blocks overflow their extent"));
        }
        let mut records = vec![0u8; stored as usize];
        read_exact_at(&self.file, &mut records, entry.extent_offset)?;
        stats.bytes_read += stored;
        Ok((entry, blocks, records))
    }

    fn entry_at(&self, index: u32) -> TocEntry {
        let at = TOC_HEADER_LEN + index as usize * ENTRY_LEN;
        TocEntry::decode(&self.toc[at..at + ENTRY_LEN])
    }

    fn string_table(&self) -> &[u8] {
        let start = TOC_HEADER_LEN
            + self.toc_header.entry_count as usize * ENTRY_LEN
            + self.toc_header.block_count as usize * BLOCK_LEN;
        &self.toc[start..]
    }

    fn key_bytes(&self, entry: &TocEntry) -> Result<&[u8], PackError> {
        let table = self.string_table();
        let start = entry.key_offset as usize;
        start
            .checked_add(entry.key_len as usize)
            .and_then(|end| table.get(start..end))
            .ok_or(PackError::TocCorrupt("key outside the string table"))
    }

    /// Binary search over the sorted entries: O(log n), no per-entry parse.
    fn find(&self, key: &str) -> Result<TocEntry, PackError> {
        let (mut low, mut high) = (0u32, self.toc_header.entry_count);
        while low < high {
            let mid = low + (high - low) / 2;
            let entry = self.entry_at(mid);
            match self.key_bytes(&entry)?.cmp(key.as_bytes()) {
                Ordering::Less => low = mid + 1,
                Ordering::Greater => high = mid,
                Ordering::Equal => return Ok(entry),
            }
        }
        Err(PackError::NotFound)
    }

    /// Block records of `entry`, checked for a canonical split: every block
    /// but the last holds exactly `block_size` plaintext bytes and the sum is
    /// the entry length.
    fn blocks(&self, entry: &TocEntry) -> Result<Vec<BlockRecord>, PackError> {
        let end = entry
            .first_block
            .checked_add(entry.block_count)
            .filter(|&end| end <= self.toc_header.block_count)
            .ok_or(PackError::TocCorrupt("block range"))?;
        let base = TOC_HEADER_LEN + self.toc_header.entry_count as usize * ENTRY_LEN;
        let blocks: Vec<BlockRecord> = (entry.first_block..end)
            .map(|index| {
                let at = base + index as usize * BLOCK_LEN;
                BlockRecord::decode(&self.toc[at..at + BLOCK_LEN])
            })
            .collect();
        check_blocks(&blocks, self.header.block_size, entry.plain_len)?;
        Ok(blocks)
    }
}

/// The block records a reader accepts for one entry: every block but the
/// last holds exactly `block_size` plaintext bytes, the last is non-empty,
/// the sum is `plain_len`, stored lengths stay within [`stored_bound`], and
/// no unknown flag is set. The packager applies the same check to cached
/// records before reusing them.
pub fn check_blocks(
    blocks: &[BlockRecord],
    block_size: u32,
    plain_len: u64,
) -> Result<(), PackError> {
    let max_stored = stored_bound(block_size);
    let mut total = 0u64;
    for (i, block) in blocks.iter().enumerate() {
        let last = i + 1 == blocks.len();
        if block.plain_len == 0
            || block.plain_len > block_size
            || (!last && block.plain_len != block_size)
            || block.stored_len as u64 > max_stored
            || block.flags & !format::BLOCK_FLAG_COMPRESSED != 0
        {
            return Err(PackError::TocCorrupt("block record"));
        }
        total += block.plain_len as u64;
    }
    if total != plain_len {
        return Err(PackError::TocCorrupt("entry length"));
    }
    Ok(())
}

/// What a sealed entry's blocks are bound to.
pub struct EntryContext<'a> {
    pub pack_id: &'a [u8; 16],
    pub block_size: u32,
    pub key_generation: u32,
    /// `Some` for encrypted packs.
    pub key: Option<&'a ContentKey>,
}

/// Verify and decode the stored `records` of one entry: per block, the
/// record hash, then AEAD authentication, then bounded decompression; then
/// the plaintext hash of the whole entry. Nothing unverified is returned.
/// `blocks` must already have passed the reader's canonical-split checks.
pub fn decode_records(
    context: &EntryContext<'_>,
    entry_key: &[u8],
    blocks: &[BlockRecord],
    records: &[u8],
    plain_hash: &[u8; 32],
    stats: &mut ReadStats,
) -> Result<Vec<u8>, PackError> {
    let plain_len: u64 = blocks.iter().map(|block| block.plain_len as u64).sum();
    let mut out = Vec::new();
    usize::try_from(plain_len)
        .ok()
        .and_then(|len| out.try_reserve_exact(len).ok())
        .ok_or(PackError::TooLarge { len: plain_len })?;
    let entry_key_id = crate::hash::entry_key_id(entry_key);
    let mut position = 0usize;
    for (index, block) in blocks.iter().enumerate() {
        let index = index as u32;
        let record = position
            .checked_add(block.stored_len as usize)
            .and_then(|end| records.get(position..end))
            .ok_or(PackError::TocCorrupt("records shorter than their blocks"))?;
        position += record.len();
        if format::record_hash(record) != block.record_hash {
            return Err(PackError::BlockHashMismatch { block: index });
        }
        let opened;
        let payload: &[u8] = match context.key {
            Some(key) => {
                let started = Instant::now();
                let aad = format::block_aad(
                    context.pack_id,
                    &entry_key_id,
                    index,
                    context.key_generation,
                    block.plain_len,
                    block.flags,
                );
                opened = seal::open(key, &aad, record)
                    .ok_or(PackError::BlockAuthFailed { block: index })?;
                stats.auth_ns += started.elapsed().as_nanos() as u64;
                &opened
            }
            None => record,
        };
        if block.compressed() {
            let started = Instant::now();
            decompress_into(payload, block.plain_len, context.block_size, &mut out)
                .ok_or(PackError::Decompress { block: index })?;
            stats.decompress_ns += started.elapsed().as_nanos() as u64;
        } else {
            if payload.len() != block.plain_len as usize {
                return Err(PackError::TocCorrupt("stored block length"));
            }
            out.extend_from_slice(payload);
        }
    }
    if position != records.len() {
        return Err(PackError::TocCorrupt("trailing record bytes"));
    }
    if &crate::hash::content(&out) != plain_hash {
        return Err(PackError::PlaintextHashMismatch);
    }
    Ok(out)
}

/// Largest stored record a block of `block_size` plaintext bytes can need:
/// a zstd frame never exceeds the input by more than a small bound, and a
/// block the packager could not shrink is stored raw.
pub fn stored_bound(block_size: u32) -> u64 {
    block_size as u64 + (block_size as u64 >> 7) + 1024 + SEAL_OVERHEAD as u64
}

fn check_header(header: &Header, raw: &[u8]) -> Result<ResourceClass, PackError> {
    if header.flags & !KNOWN_FLAGS != 0 {
        return Err(PackError::BadHeader("unknown flags"));
    }
    if !Header::reserved_is_zero(raw) {
        return Err(PackError::BadHeader("reserved bytes"));
    }
    let class = ResourceClass::from_byte(header.class).ok_or(PackError::BadHeader("class"))?;
    if !(MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&header.block_size)
        || !header.block_size.is_power_of_two()
    {
        return Err(PackError::BadHeader("block size"));
    }
    let encrypted = header.encrypted();
    if encrypted == KeyId(header.key_id).is_none() {
        return Err(PackError::BadHeader("key id"));
    }
    if encrypted && class == ResourceClass::EarlySplash {
        return Err(PackError::BadHeader(
            "EarlySplash packs cannot be encrypted",
        ));
    }
    let signed = header.signed();
    if !signed && (header.publisher_key_id != [0; 8] || header.signature != [0; 64]) {
        return Err(PackError::BadHeader("signature fields on an unsigned pack"));
    }
    if signed && header.publisher_key_id == [0; 8] {
        return Err(PackError::BadHeader(
            "signed pack without a publisher key id",
        ));
    }
    Ok(class)
}

/// Decode one zstd frame of exactly `plain_len` bytes. The window is capped
/// near the block size (a block never needs more), so a crafted frame cannot
/// make the decoder reserve the zstd default of 100 MB.
fn decompress_into(
    payload: &[u8],
    plain_len: u32,
    block_size: u32,
    out: &mut Vec<u8>,
) -> Option<()> {
    let max_window = (block_size as u64 * 2).max(1 << 17);
    let decoder =
        ruzstd::decoding::StreamingDecoder::new_with_max_window_size(payload, max_window).ok()?;
    let start = out.len();
    decoder.take(plain_len as u64 + 1).read_to_end(out).ok()?;
    (out.len() - start == plain_len as usize).then_some(())
}

fn io_error(error: io::Error) -> PackError {
    PackError::Io(error.to_string())
}

fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> Result<(), PackError> {
    let result = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            file.read_exact_at(buf, offset)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            let mut done = 0;
            let mut result = Ok(());
            while done < buf.len() {
                match file.seek_read(&mut buf[done..], offset + done as u64) {
                    Ok(0) => {
                        result = Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                        break;
                    }
                    Ok(n) => done += n,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        result = Err(error);
                        break;
                    }
                }
            }
            result
        }
    };
    result.map_err(|error| match error.kind() {
        io::ErrorKind::UnexpectedEof => PackError::Truncated,
        _ => io_error(error),
    })
}

#[cfg(test)]
pub(super) fn decompress_for_test(payload: &[u8], plain_len: u32, out: &mut Vec<u8>) -> bool {
    decompress_into(payload, plain_len, 64 * 1024, out).is_some()
}
