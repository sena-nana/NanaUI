//! Steam delta fixture (Issue #226 §5): build a baseline pack, change one
//! small thing, rebuild against the baseline, and require that only bounded
//! regions of the pack change. Gates are on deterministic counts (changed
//! 1 MiB chunks, changed bytes, re-encoded bytes), never on timings. Every
//! scenario writes its numbers to `target/performance/issue226/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use nana_package::pack::format::{HEADER_LEN, Header};
use nana_package::{
    ContentKey, KeyId, PackReader, ReadStats, ResourceClass, StaticKeys, TrustPolicy,
};
use nana_packager::cache::ArtifactCache;
use nana_packager::delta::{Delta, STEAM_CHUNK, delta};
use nana_packager::pack_build::{BuiltPack, EntrySource, InputEntry, PackInput, build_pack};

const BLOCK: u32 = 64 * 1024;

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("nana-steam-delta-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

/// Deterministic corpus: 600 resources from 1 KiB to 128 KiB, half
/// incompressible (textures, audio) and half text-like (CSS, JSON).
fn corpus() -> BTreeMap<String, Vec<u8>> {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..600)
        .map(|i| {
            let len = 1024 + (next() % (127 * 1024)) as usize;
            let text_like = i % 2 == 0;
            let bytes: Vec<u8> = (0..len)
                .map(|j| {
                    if text_like {
                        b"body { color: #333; margin: 0 auto; }\n"[j % 38]
                    } else {
                        next() as u8
                    }
                })
                .collect();
            (format!("ui/{:02}/{i:04}.bin", i % 16), bytes)
        })
        .collect()
}

fn key(generation: u32) -> (KeyId, u32, ContentKey) {
    (
        KeyId::from_name("content-main"),
        generation,
        ContentKey::from_bytes([generation as u8; 32]),
    )
}

fn build(
    files: &BTreeMap<String, Vec<u8>>,
    out: &Path,
    baseline: Option<&Path>,
    cache: Option<&ArtifactCache>,
    generation: u32,
    signer: &SigningKey,
) -> BuiltPack {
    build_pack(
        PackInput {
            application_id: "dev.nana.steam-fixture",
            name: "ui",
            class: ResourceClass::Protected,
            entries: files
                .iter()
                .map(|(key, bytes)| InputEntry {
                    key: key.clone(),
                    source: EntrySource::Bytes(bytes.clone()),
                })
                .collect(),
            block_size: BLOCK,
            zstd_level: Some(3),
            encryption: Some(key(generation)),
            signer: Some(signer),
            baseline,
            cache,
            compact: false,
            max_free_ratio: 0.25,
        },
        out,
    )
    .unwrap()
}

/// Chunks a byte range `[offset, offset + len)` touches.
fn chunks_spanned(offset: u64, len: u64) -> u64 {
    if len == 0 {
        return 0;
    }
    let chunk = STEAM_CHUNK as u64;
    (offset + len - 1) / chunk - offset / chunk + 1
}

/// Where an entry lives in a pack.
fn extent(pack: &Path, entry: &str, generation: u32) -> (u64, u64) {
    let (id, generation, key) = key(generation);
    let keys = StaticKeys::new().with(id, generation, key);
    let reader = PackReader::open(pack, "ui", &keys, &TrustPolicy::AllowUnsigned, None).unwrap();
    let (entry, _) = reader.stored_entry(entry).unwrap();
    (entry.extent_offset, entry.extent_capacity)
}

fn verify_all(pack: &Path, files: &BTreeMap<String, Vec<u8>>, generation: u32) {
    let (id, generation, key) = key(generation);
    let keys = StaticKeys::new().with(id, generation, key);
    let reader = PackReader::open(pack, "ui", &keys, &TrustPolicy::AllowUnsigned, None).unwrap();
    assert_eq!(reader.entry_count() as usize, files.len());
    for (name, bytes) in files {
        assert_eq!(
            &reader
                .read(name, u64::MAX, &mut ReadStats::default())
                .unwrap(),
            bytes,
            "{name}"
        );
    }
}

#[derive(serde::Serialize)]
struct ScenarioReport<'a> {
    scenario: &'a str,
    changed_source_bytes: u64,
    delta: Delta,
    encoded_bytes: u64,
    reused_bytes: u64,
    pack_bytes: u64,
    toc_stored_bytes: u64,
    key_rotation: bool,
    generation_ms: u64,
    warnings: &'a [String],
}

fn record(scenario: &str, changed_source_bytes: u64, delta: Delta, built: &BuiltPack) {
    let report = ScenarioReport {
        scenario,
        changed_source_bytes,
        delta,
        encoded_bytes: built.report.encoded_bytes,
        reused_bytes: built.report.reused_bytes,
        pack_bytes: built.report.pack_bytes,
        toc_stored_bytes: built.report.toc_stored_bytes,
        key_rotation: built.report.key_rotation,
        generation_ms: built.report.generation_ms,
        warnings: &built.report.warnings,
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/performance/issue226");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("delta-{scenario}.json")),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "{scenario}: {}/{} chunks, {} changed bytes, {} B download, {} B re-encoded",
        delta.changed_chunks,
        delta.total_chunks,
        delta.changed_bytes,
        delta.steam_delta_bytes,
        built.report.encoded_bytes
    );
}

/// The bound every small-change scenario must meet: the header's chunk,
/// the chunks of every extent that changed, and the TOC's chunks, old and
/// new. Nothing else may differ.
fn assert_bounded(
    scenario: &str,
    old: &[u8],
    new: &[u8],
    regions: &[(u64, u64)],
    built: &BuiltPack,
    d: &Delta,
) {
    let old_header = Header::decode(old).unwrap();
    let new_header = &built.header;
    let mut bound = chunks_spanned(0, HEADER_LEN as u64);
    for (offset, len) in regions {
        bound += chunks_spanned(*offset, *len);
    }
    bound += chunks_spanned(new_header.toc_offset, new_header.toc_stored_len);
    bound += chunks_spanned(old_header.toc_offset, old_header.toc_stored_len);
    assert!(
        d.changed_chunks <= bound,
        "{scenario}: {} changed chunks, bound {bound}",
        d.changed_chunks
    );
    let region_bytes: u64 = regions.iter().map(|(_, len)| len).sum();
    assert!(
        d.changed_bytes
            <= region_bytes + new_header.toc_stored_len + old_header.toc_stored_len + 256,
        "{scenario}: {} changed bytes",
        d.changed_bytes
    );
    // The pack is well over 10 chunks; a bounded change is a small share.
    assert!(
        d.total_chunks >= 10,
        "corpus too small: {} chunks",
        d.total_chunks
    );
    let ratio = d.steam_delta_bytes as f64 / new.len() as f64;
    assert!(ratio <= 0.25, "{scenario}: delta ratio {ratio:.3}");
    let _ = old;
}

#[test]
fn steam_delta_is_bounded_for_small_changes() {
    let dir = scratch("gates");
    let signer = SigningKey::from_bytes(&[0x11; 32]);
    let files = corpus();
    let v1 = dir.0.join("v1.nrpack");
    let first = build(&files, &v1, None, None, 1, &signer);
    // S6: without a baseline the builder says the delta will be full.
    assert!(
        first
            .report
            .warnings
            .iter()
            .any(|w| w.contains("no baseline"))
    );
    let v1_bytes = std::fs::read(&v1).unwrap();

    // S0: nothing changed.
    let v0 = dir.0.join("v0.nrpack");
    let same = build(&files, &v0, Some(&v1), None, 1, &signer);
    let d = delta(&v1_bytes, &std::fs::read(&v0).unwrap(), STEAM_CHUNK);
    record("s0-unchanged", 0, d, &same);
    assert_eq!(d.changed_chunks, 0);
    assert_eq!(same.report.encoded_bytes, 0);

    // S1: one small resource changes, same size.
    let target = "ui/07/0103.bin";
    let mut s1 = files.clone();
    let len = s1[target].len();
    s1.insert(target.into(), (0..len).map(|i| (i * 7) as u8).collect());
    let v2 = dir.0.join("v2.nrpack");
    let built = build(&s1, &v2, Some(&v1), None, 1, &signer);
    let v2_bytes = std::fs::read(&v2).unwrap();
    let d = delta(&v1_bytes, &v2_bytes, STEAM_CHUNK);
    record("s1-one-changed", len as u64, d, &built);
    let region = extent(&v2, target, 1);
    assert_bounded("s1", &v1_bytes, &v2_bytes, &[region], &built, &d);
    assert!(built.report.encoded_bytes <= region.1);
    verify_all(&v2, &s1, 1);

    // S2: the resource grows past its extent and moves.
    let mut s2 = files.clone();
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let grown: Vec<u8> = (0..300 * 1024)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect();
    let old_region = extent(&v1, target, 1);
    s2.insert(target.into(), grown.clone());
    let v3 = dir.0.join("v3.nrpack");
    let built = build(&s2, &v3, Some(&v1), None, 1, &signer);
    let v3_bytes = std::fs::read(&v3).unwrap();
    let d = delta(&v1_bytes, &v3_bytes, STEAM_CHUNK);
    record("s2-grown", grown.len() as u64, d, &built);
    assert_eq!(built.report.relocated, 1);
    assert_bounded(
        "s2",
        &v1_bytes,
        &v3_bytes,
        &[old_region, extent(&v3, target, 1)],
        &built,
        &d,
    );
    verify_all(&v3, &s2, 1);

    // S3: one new resource.
    let mut s3 = files.clone();
    let added: Vec<u8> = (0..16 * 1024).map(|i| (i * 3) as u8).collect();
    s3.insert("ui/new/added.bin".into(), added.clone());
    let v4 = dir.0.join("v4.nrpack");
    let built = build(&s3, &v4, Some(&v1), None, 1, &signer);
    let v4_bytes = std::fs::read(&v4).unwrap();
    let d = delta(&v1_bytes, &v4_bytes, STEAM_CHUNK);
    record("s3-added", added.len() as u64, d, &built);
    assert_eq!(built.report.added, 1);
    assert_bounded(
        "s3",
        &v1_bytes,
        &v4_bytes,
        &[extent(&v4, "ui/new/added.bin", 1)],
        &built,
        &d,
    );
    verify_all(&v4, &s3, 1);

    // S4: one resource removed; its extent is zeroed.
    let mut s4 = files.clone();
    let removed_region = extent(&v1, target, 1);
    s4.remove(target);
    let v5 = dir.0.join("v5.nrpack");
    let built = build(&s4, &v5, Some(&v1), None, 1, &signer);
    let v5_bytes = std::fs::read(&v5).unwrap();
    let d = delta(&v1_bytes, &v5_bytes, STEAM_CHUNK);
    record("s4-removed", 0, d, &built);
    assert_eq!(built.report.removed, 1);
    assert_bounded("s4", &v1_bytes, &v5_bytes, &[removed_region], &built, &d);
    verify_all(&v5, &s4, 1);

    // S5: key rotation — allowed to change everything, and says so.
    let v6 = dir.0.join("v6.nrpack");
    let built = build(&files, &v6, Some(&v1), None, 2, &signer);
    let d = delta(&v1_bytes, &std::fs::read(&v6).unwrap(), STEAM_CHUNK);
    record("s5-key-rotation", 0, d, &built);
    assert!(built.report.key_rotation);
    assert!(d.changed_chunks > d.total_chunks / 2);
    verify_all(&v6, &files, 2);
}

/// A CI runner without the previous package but with a persistent cache
/// still reproduces every record byte for byte (only the sealed TOC, with a
/// fresh nonce, differs).
#[test]
fn cache_without_baseline_keeps_records_stable() {
    let dir = scratch("cache");
    let signer = SigningKey::from_bytes(&[0x22; 32]);
    let cache = ArtifactCache::new(&dir.0.join("cache"));
    let files = corpus();
    let a = dir.0.join("a.nrpack");
    let b = dir.0.join("b.nrpack");
    build(&files, &a, None, Some(&cache), 1, &signer);
    let built = build(&files, &b, None, Some(&cache), 1, &signer);
    assert_eq!(built.report.encoded_bytes, 0);
    let (a, b) = (std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
    let toc = built.header.toc_offset as usize;
    assert_eq!(a[HEADER_LEN..toc], b[HEADER_LEN..toc]);
    let d = delta(&a, &b, STEAM_CHUNK);
    record("s0b-cache-only", 0, d, &built);
    assert!(d.changed_chunks <= 1 + chunks_spanned(toc as u64, built.header.toc_stored_len) * 2);
}
