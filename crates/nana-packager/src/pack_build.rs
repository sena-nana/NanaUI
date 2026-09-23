//! Building one `.nrpack` deterministically and delta-friendly.
//!
//! Given the previous build (`baseline`) the builder keeps every unchanged
//! entry's stored bytes at the same offset, rewrites a changed entry in
//! place when it still fits its extent, and places new or grown entries in
//! freed space or at the end. Stored bytes of unchanged entries come from the
//! baseline or the artifact cache, verified before reuse, so an unchanged
//! input never gets a fresh nonce and the pack changes only where its
//! content did (Issue #226 §5, §6).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use ed25519_dalek::{Signer, SigningKey};
use nana_package::pack::format::{
    self, BLOCK_FLAG_COMPRESSED, BlockRecord, CODEC_STORED, CODEC_ZSTD, EXTENT_ALIGN,
    FLAG_ENCRYPTED, FLAG_SIGNED, FORMAT_VERSION, HEADER_LEN, Header, NONCE_LEN, PAGE,
    SEAL_OVERHEAD, TocEntry, TocHeader,
};
use nana_package::pack::{EntryContext, ReadStats, check_blocks, decode_records, seal};
use nana_package::{
    ContentKey, KeyId, NoKeys, PackReader, PublisherKey, ResourceClass, StaticKeys, TrustPolicy,
};
use serde::Serialize;

use crate::cache::ArtifactCache;

/// Where an entry's bytes come from.
#[derive(Debug, Clone)]
pub enum EntrySource {
    File(PathBuf),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct InputEntry {
    pub key: String,
    pub source: EntrySource,
}

pub struct PackInput<'a> {
    pub application_id: &'a str,
    pub name: &'a str,
    pub class: ResourceClass,
    /// Any order; the builder sorts by key bytes.
    pub entries: Vec<InputEntry>,
    pub block_size: u32,
    /// zstd level, or `None` to store blocks uncompressed.
    pub zstd_level: Option<i32>,
    pub encryption: Option<(KeyId, u32, ContentKey)>,
    pub signer: Option<&'a SigningKey>,
    /// The previously shipped pack, whose layout and bytes to keep.
    pub baseline: Option<&'a Path>,
    pub cache: Option<&'a ArtifactCache>,
    /// Ignore the baseline layout and lay entries out in key order.
    pub compact: bool,
    pub max_free_ratio: f64,
}

/// What a build did. Counts, not timings, are what CI gates on.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PackReport {
    pub name: String,
    pub entries: u32,
    pub pack_bytes: u64,
    pub toc_stored_bytes: u64,
    pub source_bytes: u64,
    /// Source bytes of entries that are new or changed since the baseline
    /// (all of them without one).
    pub changed_source_bytes: u64,
    /// Bytes compressed and sealed in this run (the local rebuild cost).
    pub encoded_bytes: u64,
    /// Stored bytes reused from the baseline or the cache.
    pub reused_bytes: u64,
    pub reused_from_baseline: u32,
    pub cache_hits: u32,
    pub cache_misses: u32,
    pub kept_in_place: u32,
    pub rewritten_in_place: u32,
    pub relocated: u32,
    pub added: u32,
    pub removed: u32,
    pub free_bytes: u64,
    pub compacted: bool,
    /// The key id or generation differs from the baseline: every stored
    /// byte is new, and a large delta is expected.
    pub key_rotation: bool,
    pub toc_reused: bool,
    pub generation_ms: u64,
    pub warnings: Vec<String>,
}

/// Header facts the manifest records.
#[derive(Debug, Clone)]
pub struct BuiltPack {
    pub header: Header,
    pub report: PackReport,
}

struct EncodedEntry {
    key: String,
    plain_len: u64,
    plain_hash: [u8; 32],
    blocks: Vec<BlockRecord>,
    records: Vec<u8>,
}

struct BaselineInfo {
    reader: PackReader,
    /// Every baseline entry, decoded once (extent, hash, codec).
    entries: BTreeMap<String, TocEntry>,
    /// Record reuse needs identical sealing parameters.
    records_reusable: bool,
}

impl BaselineInfo {
    fn data_end(&self) -> u64 {
        self.reader.header().data_end
    }
}

/// How entries landed relative to the baseline.
#[derive(Default)]
struct Placement {
    layout: Vec<(u64, u64)>,
    kept_in_place: u32,
    rewritten_in_place: u32,
    relocated: u32,
    added: u32,
}

pub fn build_pack(input: PackInput<'_>, out: &Path) -> Result<BuiltPack, String> {
    let started = Instant::now();
    let pack_id = nana_package::hash::pack_id(input.application_id, input.name);
    let mut report = PackReport {
        name: input.name.to_owned(),
        ..PackReport::default()
    };
    let (codec, level) = match input.zstd_level {
        Some(level) => (CODEC_ZSTD, level),
        None => (CODEC_STORED, 0),
    };
    let (key_id, key_generation) = input
        .encryption
        .as_ref()
        .map_or((KeyId::NONE, 0), |(id, generation, _)| (*id, *generation));
    let content_key = input.encryption.as_ref().map(|(_, _, key)| key);
    let encrypted = content_key.is_some();
    if input.class == ResourceClass::EarlySplash && encrypted {
        return Err(format!(
            "EarlySplash pack `{}` cannot be encrypted",
            input.name
        ));
    }

    let baseline = open_baseline(&input, &pack_id, key_id, key_generation, &mut report)?;

    // Encode (or reuse) every entry.
    let mut entries = input.entries;
    entries.sort_by(|a, b| a.key.as_bytes().cmp(b.key.as_bytes()));
    for pair in entries.windows(2) {
        if pair[0].key == pair[1].key {
            return Err(format!("duplicate entry `{}`", pair[0].key));
        }
    }
    let context = EntryContext {
        pack_id: &pack_id,
        block_size: input.block_size,
        key_generation,
        key: content_key,
    };
    let mut encoded = Vec::with_capacity(entries.len());
    let mut compression_changed = false;
    for entry in &entries {
        let data: std::borrow::Cow<'_, [u8]> = match &entry.source {
            EntrySource::Bytes(bytes) => bytes.into(),
            EntrySource::File(path) => std::fs::read(path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?
                .into(),
        };
        report.source_bytes += data.len() as u64;
        let plain_hash = nana_package::hash::content(&data);
        match baseline.as_ref().and_then(|b| b.entries.get(&entry.key)) {
            Some(old) if old.plain_hash == plain_hash => {
                if (old.codec, old.codec_level) != (codec, level) && !compression_changed {
                    compression_changed = true;
                    report.warnings.push(
                        "compression settings changed since the baseline: unchanged entries are \
                         re-encoded (large delta)"
                            .into(),
                    );
                }
            }
            _ => report.changed_source_bytes += data.len() as u64,
        }

        if let Some(reused) =
            reuse_from_baseline(&baseline, &context, &entry.key, &plain_hash, codec, level)
        {
            report.reused_from_baseline += 1;
            report.reused_bytes += reused.records.len() as u64;
            encoded.push(reused);
            continue;
        }
        let cache_key = input.cache.map(|_| {
            ArtifactCache::key(
                &pack_id,
                &entry.key,
                &plain_hash,
                input.zstd_level,
                input.block_size,
                key_id,
                key_generation,
            )
        });
        if let (Some(cache), Some(cache_key)) = (input.cache, &cache_key) {
            if let Some((blocks, records)) = cache.get(cache_key)
                && check_blocks(&blocks, input.block_size, data.len() as u64).is_ok()
                && decode_records(
                    &context,
                    entry.key.as_bytes(),
                    &blocks,
                    &records,
                    &plain_hash,
                    &mut ReadStats::default(),
                )
                .is_ok()
            {
                report.cache_hits += 1;
                report.reused_bytes += records.len() as u64;
                encoded.push(EncodedEntry {
                    key: entry.key.clone(),
                    plain_len: data.len() as u64,
                    plain_hash,
                    blocks,
                    records,
                });
                continue;
            }
            report.cache_misses += 1;
        }
        let fresh = encode_entry(&context, &entry.key, &data, plain_hash, input.zstd_level)?;
        report.encoded_bytes += fresh.records.len() as u64;
        if let (Some(cache), Some(cache_key)) = (input.cache, &cache_key) {
            cache.put(cache_key, &fresh.blocks, &fresh.records)?;
        }
        encoded.push(fresh);
    }
    report.entries = encoded.len() as u32;

    // Lay out extents.
    let mut placement = place(&encoded, baseline.as_ref(), input.compact);
    let used = |layout: &[(u64, u64)]| layout.iter().map(|(_, capacity)| *capacity).sum::<u64>();
    let span = layout_end(&placement.layout, baseline.as_ref(), input.compact)
        .saturating_sub(HEADER_LEN as u64);
    if baseline.is_some() && !input.compact && span > 0 {
        let free_ratio = (span - used(&placement.layout).min(span)) as f64 / span as f64;
        if free_ratio > input.max_free_ratio {
            report.warnings.push(format!(
                "free space {:.0}% exceeds max_free_ratio {:.0}%: compacting (large delta)",
                free_ratio * 100.0,
                input.max_free_ratio * 100.0
            ));
            placement = place(&encoded, None, true);
            report.compacted = true;
        }
    }
    report.compacted |= input.compact && baseline.is_some();
    report.kept_in_place = placement.kept_in_place;
    report.rewritten_in_place = placement.rewritten_in_place;
    report.relocated = placement.relocated;
    report.added = placement.added;
    let layout = placement.layout;
    let data_end = layout_end(&layout, baseline.as_ref(), report.compacted);
    report.free_bytes = data_end
        .saturating_sub(HEADER_LEN as u64)
        .saturating_sub(used(&layout));
    if let Some(baseline) = &baseline {
        report.removed = baseline
            .entries
            .keys()
            .filter(|key| {
                encoded
                    .binary_search_by(|e| e.key.as_bytes().cmp(key.as_bytes()))
                    .is_err()
            })
            .count() as u32;
    }

    // TOC.
    let mut toc = Vec::new();
    let string_table_len: usize = encoded.iter().map(|e| e.key.len()).sum();
    let block_count: usize = encoded.iter().map(|e| e.blocks.len()).sum();
    TocHeader {
        entry_count: encoded.len() as u32,
        string_table_len: u32::try_from(string_table_len)
            .map_err(|_| "TOC string table too large")?,
        block_count: u32::try_from(block_count).map_err(|_| "too many blocks")?,
    }
    .encode(&mut toc);
    let (mut key_offset, mut first_block) = (0u32, 0u32);
    for (entry, (offset, capacity)) in encoded.iter().zip(&layout) {
        TocEntry {
            key_offset,
            key_len: entry.key.len() as u32,
            codec,
            codec_level: level,
            plain_len: entry.plain_len,
            plain_hash: entry.plain_hash,
            extent_offset: *offset,
            extent_capacity: *capacity,
            first_block,
            block_count: entry.blocks.len() as u32,
        }
        .encode(&mut toc);
        key_offset += entry.key.len() as u32;
        first_block += entry.blocks.len() as u32;
    }
    for entry in &encoded {
        for block in &entry.blocks {
            block.encode(&mut toc);
        }
    }
    for entry in &encoded {
        toc.extend_from_slice(entry.key.as_bytes());
    }

    let toc_offset = format::align_up(data_end, PAGE);
    let toc_stored_len = toc.len() as u64 + if encrypted { SEAL_OVERHEAD as u64 } else { 0 };
    let publisher = input.signer.map(|signer| {
        PublisherKey::from_bytes(signer.verifying_key().as_bytes()).expect("valid public key")
    });
    let mut header = Header {
        version: FORMAT_VERSION,
        flags: if encrypted { FLAG_ENCRYPTED } else { 0 }
            | if publisher.is_some() { FLAG_SIGNED } else { 0 },
        pack_id,
        block_size: input.block_size,
        class: input.class.to_byte(),
        key_id: key_id.0,
        key_generation,
        entry_count: encoded.len() as u32,
        toc_offset,
        toc_stored_len,
        toc_plain_len: toc.len() as u64,
        data_end,
        toc_hash: [0; 32],
        publisher_key_id: publisher.as_ref().map_or([0; 8], |p| p.id().0),
        signature: [0; 64],
    };
    let stored_toc = match content_key {
        None => toc,
        Some(key) => {
            let aad = format::toc_aad(&header.encode());
            // An unchanged TOC under an unchanged header keeps its bytes (and
            // nonce), so a no-op rebuild is byte-identical.
            match reuse_toc(baseline.as_ref(), &header, &toc) {
                Some(stored) => {
                    report.toc_reused = true;
                    stored
                }
                None => seal::seal(key, &random_nonce()?, &aad, &toc),
            }
        }
    };
    debug_assert_eq!(stored_toc.len() as u64, toc_stored_len);
    header.toc_hash = nana_package::hash::content(&stored_toc);
    if let Some(signer) = input.signer {
        header.signature = signer
            .sign(&Header::signing_message(&header.encode()))
            .to_bytes();
    }
    report.toc_stored_bytes = toc_stored_len;
    report.pack_bytes = toc_offset + toc_stored_len;
    drop(baseline);

    write_pack(out, &header, &encoded, &layout, &stored_toc)?;
    report.generation_ms = started.elapsed().as_millis() as u64;
    Ok(BuiltPack { header, report })
}

fn open_baseline(
    input: &PackInput<'_>,
    pack_id: &[u8; 16],
    key_id: KeyId,
    key_generation: u32,
    report: &mut PackReport,
) -> Result<Option<BaselineInfo>, String> {
    let Some(path) = input.baseline else {
        report
            .warnings
            .push("no baseline: every entry is laid out fresh (expect a full delta)".into());
        return Ok(None);
    };
    if !path.exists() {
        report.warnings.push(format!(
            "baseline {} does not exist: new pack (expect a full delta)",
            path.display()
        ));
        return Ok(None);
    }
    // Only the current key is known; a baseline sealed under another
    // generation cannot be opened, and nothing of it can be reused.
    let keys: Box<dyn nana_package::KeyProvider> = match &input.encryption {
        Some((id, generation, key)) => {
            Box::new(StaticKeys::new().with(*id, *generation, key.clone()))
        }
        None => Box::new(NoKeys),
    };
    let reader = match PackReader::open(
        path,
        input.name,
        keys.as_ref(),
        &TrustPolicy::AllowUnsigned,
        None,
    ) {
        Ok(reader) => reader,
        Err(error) => {
            // Only the header is needed to tell a rotation from damage.
            let header = std::fs::File::open(path).ok().and_then(|file| {
                let mut bytes = Vec::with_capacity(HEADER_LEN);
                file.take(HEADER_LEN as u64).read_to_end(&mut bytes).ok()?;
                Header::decode(&bytes)
            });
            if header.is_some_and(|h| h.key_id != key_id.0 || h.key_generation != key_generation) {
                report.key_rotation = true;
                report.warnings.push(
                    "content key rotated since the baseline: every stored byte changes".into(),
                );
            } else {
                report
                    .warnings
                    .push(format!("baseline unusable ({error}): new layout"));
            }
            return Ok(None);
        }
    };
    let header = reader.header().clone();
    if &header.pack_id != pack_id || header.block_size != input.block_size {
        report.warnings.push(
            "baseline belongs to another pack or block size: new layout (expect a full delta)"
                .into(),
        );
        return Ok(None);
    }
    let records_reusable = header.key_id == key_id.0 && header.key_generation == key_generation;
    if !records_reusable {
        report.key_rotation = true;
        report
            .warnings
            .push("encryption changed since the baseline: every stored byte changes".into());
    }
    let mut entries = BTreeMap::new();
    for key in reader
        .keys()
        .map_err(|error| format!("baseline: {error}"))?
    {
        let (entry, _) = reader
            .stored_entry(&key)
            .map_err(|error| format!("baseline entry {key}: {error}"))?;
        entries.insert(key, entry);
    }
    Ok(Some(BaselineInfo {
        reader,
        entries,
        records_reusable,
    }))
}

fn reuse_from_baseline(
    baseline: &Option<BaselineInfo>,
    context: &EntryContext<'_>,
    key: &str,
    plain_hash: &[u8; 32],
    codec: u8,
    level: i32,
) -> Option<EncodedEntry> {
    let baseline = baseline.as_ref().filter(|b| b.records_reusable)?;
    let old = baseline.entries.get(key)?;
    if &old.plain_hash != plain_hash || old.codec != codec || old.codec_level != level {
        return None;
    }
    let (entry, blocks, records) = baseline.reader.stored_records(key).ok()?;
    // Verified exactly as a read would be before reuse.
    decode_records(
        context,
        key.as_bytes(),
        &blocks,
        &records,
        plain_hash,
        &mut ReadStats::default(),
    )
    .ok()?;
    Some(EncodedEntry {
        key: key.to_owned(),
        plain_len: entry.plain_len,
        plain_hash: *plain_hash,
        blocks,
        records,
    })
}

fn reuse_toc(baseline: Option<&BaselineInfo>, header: &Header, toc: &[u8]) -> Option<Vec<u8>> {
    let baseline = baseline.filter(|b| b.records_reusable)?;
    let old = baseline.reader.header();
    let new_encoded = header.encode();
    if old.encode()[..format::TOC_AAD_HEADER_LEN] != new_encoded[..format::TOC_AAD_HEADER_LEN]
        || baseline.reader.toc_plaintext() != toc
    {
        return None;
    }
    // Re-read from the file: check it is still the TOC the reader verified.
    let stored = baseline
        .reader
        .read_stored(old.toc_offset, old.toc_stored_len)
        .ok()?;
    (nana_package::hash::content(&stored) == old.toc_hash).then_some(stored)
}

fn encode_entry(
    context: &EntryContext<'_>,
    key: &str,
    data: &[u8],
    plain_hash: [u8; 32],
    zstd_level: Option<i32>,
) -> Result<EncodedEntry, String> {
    let entry_key_id = nana_package::hash::entry_key_id(key.as_bytes());
    let mut blocks = Vec::new();
    let mut records = Vec::new();
    for (index, chunk) in data.chunks(context.block_size as usize).enumerate() {
        let compressed = match zstd_level {
            Some(level) => {
                let frame = zstd::bulk::compress(chunk, level)
                    .map_err(|error| format!("zstd failed on {key}: {error}"))?;
                (frame.len() < chunk.len()).then_some(frame)
            }
            None => None,
        };
        let flags = if compressed.is_some() {
            BLOCK_FLAG_COMPRESSED
        } else {
            0
        };
        let payload = compressed.as_deref().unwrap_or(chunk);
        let record = match context.key {
            Some(content_key) => {
                let aad = format::block_aad(
                    context.pack_id,
                    &entry_key_id,
                    index as u32,
                    context.key_generation,
                    chunk.len() as u32,
                    flags,
                );
                seal::seal(content_key, &random_nonce()?, &aad, payload)
            }
            None => payload.to_vec(),
        };
        blocks.push(BlockRecord {
            stored_len: record.len() as u32,
            plain_len: chunk.len() as u32,
            flags,
            record_hash: format::record_hash(&record),
        });
        records.extend_from_slice(&record);
    }
    Ok(EncodedEntry {
        key: key.to_owned(),
        plain_len: data.len() as u64,
        plain_hash,
        blocks,
        records,
    })
}

fn random_nonce() -> Result<[u8; NONCE_LEN], String> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|error| format!("system RNG failed: {error}"))?;
    Ok(nonce)
}

fn extent_size(entry: &EncodedEntry) -> u64 {
    format::align_up(entry.records.len() as u64, EXTENT_ALIGN)
}

/// (offset, capacity) per entry, in entry order, and how each landed.
fn place(entries: &[EncodedEntry], baseline: Option<&BaselineInfo>, compact: bool) -> Placement {
    let first = HEADER_LEN as u64;
    let Some(baseline) = baseline.filter(|_| !compact) else {
        let mut at = first;
        let layout = entries
            .iter()
            .map(|entry| {
                let size = extent_size(entry);
                let placed = (at, size);
                at += size;
                placed
            })
            .collect();
        return Placement {
            layout,
            ..Placement::default()
        };
    };

    let mut report = Placement::default();
    let mut layout = vec![(0u64, 0u64); entries.len()];
    let mut pending = Vec::new();
    let mut occupied: Vec<(u64, u64)> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let size = entry.records.len() as u64;
        match baseline.entries.get(&entry.key) {
            Some(old) if size <= old.extent_capacity => {
                let (offset, capacity) = (old.extent_offset, old.extent_capacity);
                layout[index] = (offset, capacity);
                if capacity > 0 {
                    occupied.push((offset, offset + capacity));
                }
                if old.plain_hash == entry.plain_hash {
                    report.kept_in_place += 1;
                } else {
                    report.rewritten_in_place += 1;
                }
            }
            Some(_) => {
                report.relocated += 1;
                pending.push(index);
            }
            None => {
                report.added += 1;
                pending.push(index);
            }
        }
    }
    // Free gaps between kept extents, within the baseline data region.
    occupied.sort_unstable();
    let mut gaps = Vec::new();
    let mut cursor = first;
    for (start, end) in &occupied {
        if *start > cursor {
            gaps.push((cursor, *start));
        }
        cursor = cursor.max(*end);
    }
    if baseline.data_end() > cursor {
        gaps.push((cursor, baseline.data_end()));
    }
    let mut end = cursor.max(baseline.data_end());
    for index in pending {
        let size = extent_size(&entries[index]);
        if size == 0 {
            layout[index] = (first, 0);
            continue;
        }
        let slot = gaps
            .iter_mut()
            .find(|(start, stop)| format::align_up(*start, EXTENT_ALIGN) + size <= *stop);
        match slot {
            Some(gap) => {
                let at = format::align_up(gap.0, EXTENT_ALIGN);
                layout[index] = (at, size);
                gap.0 = at + size;
            }
            None => {
                let at = format::align_up(end, EXTENT_ALIGN);
                layout[index] = (at, size);
                end = at + size;
            }
        }
    }
    report.layout = layout;
    report
}

fn layout_end(layout: &[(u64, u64)], baseline: Option<&BaselineInfo>, compact: bool) -> u64 {
    let placed = layout
        .iter()
        .map(|(offset, capacity)| offset + capacity)
        .max()
        .unwrap_or(HEADER_LEN as u64)
        .max(HEADER_LEN as u64);
    match baseline {
        // Keep the TOC where it was unless the data region grew.
        Some(baseline) if !compact => placed.max(baseline.data_end()),
        _ => placed,
    }
}

fn write_pack(
    out: &Path,
    header: &Header,
    entries: &[EncodedEntry],
    layout: &[(u64, u64)],
    stored_toc: &[u8],
) -> Result<(), String> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let temp = out.with_extension("nrpack.partial");
    let io = |error: std::io::Error| format!("cannot write {}: {error}", temp.display());
    let mut file = File::create(&temp).map_err(io)?;
    // Zero-filled data region; only extents are written.
    file.set_len(header.toc_offset).map_err(io)?;
    file.write_all(&header.encode()).map_err(io)?;
    for (entry, (offset, capacity)) in entries.iter().zip(layout) {
        debug_assert!(entry.records.len() as u64 <= *capacity);
        if entry.records.is_empty() {
            continue;
        }
        file.seek(SeekFrom::Start(*offset)).map_err(io)?;
        file.write_all(&entry.records).map_err(io)?;
    }
    file.seek(SeekFrom::Start(header.toc_offset)).map_err(io)?;
    file.write_all(stored_toc).map_err(io)?;
    file.sync_all().map_err(io)?;
    drop(file);
    std::fs::rename(&temp, out)
        .map_err(|error| format!("cannot move {} into place: {error}", out.display()))
}

#[cfg(test)]
mod tests;
