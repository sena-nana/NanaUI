//! Reader tests against packs built by a minimal in-test writer. The real
//! writer (layout reuse, compression, caching) is `nana-packager`'s; this one
//! only has to produce valid bytes, and to produce invalid ones on purpose.

use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer, SigningKey};

use super::format::*;
use super::*;
use crate::keys::{ContentKey, KeyId, KeyProvider, NoKeys, PublisherKey, StaticKeys, TrustPolicy};

const BLOCK: u32 = 4096;

struct Spec<'a> {
    entries: Vec<(&'a str, Vec<u8>)>,
    key: Option<(KeyId, u32, ContentKey)>,
    signer: Option<&'a SigningKey>,
    class: ResourceClass,
}

fn build(spec: &Spec<'_>) -> Vec<u8> {
    let pack_id = crate::hash::pack_id("dev.nana.test", "ui");
    let mut entries: Vec<_> = spec.entries.iter().collect();
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut data = vec![0u8; HEADER_LEN];
    let mut toc_entries = Vec::new();
    let mut blocks = Vec::new();
    let mut strings = Vec::new();
    let mut nonce_counter = 0u8;
    for (key, content) in entries {
        let offset = align_up(data.len() as u64, EXTENT_ALIGN);
        data.resize(offset as usize, 0);
        let first_block = blocks.len() as u32;
        for (index, chunk) in content.chunks(BLOCK as usize).enumerate() {
            let record = match &spec.key {
                Some((_, generation, content_key)) => {
                    nonce_counter += 1;
                    let aad = block_aad(
                        &pack_id,
                        &crate::hash::entry_key_id(key.as_bytes()),
                        index as u32,
                        *generation,
                        chunk.len() as u32,
                        0,
                    );
                    seal::seal(content_key, &[nonce_counter; 24], &aad, chunk)
                }
                None => chunk.to_vec(),
            };
            blocks.push(BlockRecord {
                stored_len: record.len() as u32,
                plain_len: chunk.len() as u32,
                flags: 0,
                record_hash: record_hash(&record),
            });
            data.extend_from_slice(&record);
        }
        toc_entries.push(TocEntry {
            key_offset: strings.len() as u32,
            key_len: key.len() as u32,
            codec: CODEC_STORED,
            codec_level: 0,
            plain_len: content.len() as u64,
            plain_hash: crate::hash::content(content),
            extent_offset: offset,
            extent_capacity: data.len() as u64 - offset,
            first_block,
            block_count: blocks.len() as u32 - first_block,
        });
        strings.extend_from_slice(key.as_bytes());
    }
    let data_end = data.len() as u64;
    let toc_offset = align_up(data_end, PAGE);
    data.resize(toc_offset as usize, 0);

    let mut toc = Vec::new();
    TocHeader {
        entry_count: toc_entries.len() as u32,
        string_table_len: strings.len() as u32,
        block_count: blocks.len() as u32,
    }
    .encode(&mut toc);
    for entry in &toc_entries {
        entry.encode(&mut toc);
    }
    for block in &blocks {
        block.encode(&mut toc);
    }
    toc.extend_from_slice(&strings);
    let toc_plain_len = toc.len() as u64;
    let (key_id, generation) = spec
        .key
        .as_ref()
        .map_or(([0; 8], 0), |(id, generation, _)| (id.0, *generation));
    let encrypted = spec.key.is_some();
    let toc_stored_len = toc_plain_len + if encrypted { SEAL_OVERHEAD as u64 } else { 0 };
    let mut header = Header {
        version: FORMAT_VERSION,
        flags: if spec.key.is_some() {
            FLAG_ENCRYPTED
        } else {
            0
        },
        pack_id,
        block_size: BLOCK,
        class: spec.class.to_byte(),
        key_id,
        key_generation: generation,
        entry_count: toc_entries.len() as u32,
        toc_offset,
        toc_stored_len,
        toc_plain_len,
        data_end,
        toc_hash: [0; 32],
        publisher_key_id: [0; 8],
        signature: [0; 64],
    };
    let stored_toc = match &spec.key {
        Some((_, _, content_key)) => {
            seal::seal(content_key, &[0xee; 24], &toc_aad(&header.encode()), &toc)
        }
        None => toc,
    };
    assert_eq!(stored_toc.len() as u64, toc_stored_len);
    header.toc_hash = crate::hash::content(&stored_toc);
    if let Some(signer) = spec.signer {
        header.flags |= FLAG_SIGNED;
        let public = PublisherKey::from_bytes(signer.verifying_key().as_bytes()).unwrap();
        header.publisher_key_id = public.id().0;
        let message = Header::signing_message(&header.encode());
        header.signature = signer.sign(&message).to_bytes();
    }
    data[..HEADER_LEN].copy_from_slice(&header.encode());
    data.extend_from_slice(&stored_toc);
    data
}

struct TempPack(PathBuf);

impl TempPack {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let dir = std::env::temp_dir().join(format!("nana-package-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.nrpack"));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempPack {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn content(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31) ^ seed)
        .collect()
}

fn plain_spec<'a>() -> Spec<'a> {
    Spec {
        entries: vec![
            ("ui/logo.png", content(1, 10_000)),
            ("fonts/a.ttf", content(2, BLOCK as usize)),
            ("ui/empty.txt", Vec::new()),
            ("ui/app.css", content(3, 17)),
        ],
        key: None,
        signer: None,
        class: ResourceClass::Protected,
    }
}

fn key() -> (KeyId, u32, ContentKey) {
    (
        KeyId::from_name("main"),
        3,
        ContentKey::from_bytes([0x42; 32]),
    )
}

fn keys() -> StaticKeys {
    let (id, generation, key) = key();
    StaticKeys::new().with(id, generation, key)
}

fn open(path: &Path, keys: &dyn KeyProvider, trust: &TrustPolicy) -> Result<PackReader, PackError> {
    PackReader::open(path, "ui", keys, trust, None)
}

fn read(reader: &PackReader, key: &str) -> Result<Vec<u8>, PackError> {
    reader.read(key, u64::MAX, &mut ReadStats::default())
}

#[test]
fn plain_pack_reads_every_entry() {
    let spec = plain_spec();
    let pack = TempPack::new("plain", &build(&spec));
    let reader = open(pack.path(), &NoKeys, &TrustPolicy::AllowUnsigned).unwrap();
    assert_eq!(reader.entry_count(), 4);
    for (key, expected) in &spec.entries {
        assert_eq!(&read(&reader, key).unwrap(), expected, "{key}");
    }
    assert_eq!(read(&reader, "ui/missing.png"), Err(PackError::NotFound));
    assert_eq!(
        reader.keys().unwrap(),
        ["fonts/a.ttf", "ui/app.css", "ui/empty.txt", "ui/logo.png"]
    );
    let mut stats = ReadStats::default();
    reader.read("ui/logo.png", u64::MAX, &mut stats).unwrap();
    assert_eq!(stats.bytes_read, 10_000);
}

#[test]
fn max_bytes_is_enforced_before_reading() {
    let pack = TempPack::new("max", &build(&plain_spec()));
    let reader = open(pack.path(), &NoKeys, &TrustPolicy::AllowUnsigned).unwrap();
    let mut stats = ReadStats::default();
    assert_eq!(
        reader.read("ui/logo.png", 9_999, &mut stats),
        Err(PackError::TooLarge { len: 10_000 })
    );
    assert_eq!(stats.bytes_read, 0);
}

#[test]
fn missing_and_foreign_files_fail_explicitly() {
    let missing = std::env::temp_dir().join("nana-package-does-not-exist.nrpack");
    assert_eq!(
        open(&missing, &NoKeys, &TrustPolicy::AllowUnsigned).unwrap_err(),
        PackError::Missing
    );
    let foreign = TempPack::new("foreign", &vec![7u8; 8192]);
    assert_eq!(
        open(foreign.path(), &NoKeys, &TrustPolicy::AllowUnsigned).unwrap_err(),
        PackError::NotAPack
    );
}

#[test]
fn truncation_anywhere_is_detected() {
    let bytes = build(&plain_spec());
    for cut in [
        HEADER_LEN - 1,
        HEADER_LEN,
        HEADER_LEN + 100,
        bytes.len() - 1,
    ] {
        let pack = TempPack::new(&format!("cut{cut}"), &bytes[..cut]);
        assert!(
            open(pack.path(), &NoKeys, &TrustPolicy::AllowUnsigned).is_err(),
            "cut at {cut}"
        );
    }
}

#[test]
fn corrupted_data_or_toc_never_returns_bytes() {
    let bytes = build(&plain_spec());
    // A data byte inside ui/logo.png's first block.
    let reader_bytes = {
        let pack = TempPack::new("probe", &bytes);
        let reader = open(pack.path(), &NoKeys, &TrustPolicy::AllowUnsigned).unwrap();
        reader.stored_entry("ui/logo.png").unwrap().0.extent_offset as usize
    };
    let mut data = bytes.clone();
    data[reader_bytes + 5] ^= 0xff;
    let pack = TempPack::new("data", &data);
    let reader = open(pack.path(), &NoKeys, &TrustPolicy::AllowUnsigned).unwrap();
    assert_eq!(
        read(&reader, "ui/logo.png"),
        Err(PackError::BlockHashMismatch { block: 0 })
    );
    // Other entries are unaffected: corruption is contained per entry.
    assert!(read(&reader, "ui/app.css").is_ok());

    let mut toc = bytes.clone();
    let last = toc.len() - 3;
    toc[last] ^= 1;
    let pack = TempPack::new("toc", &toc);
    assert_eq!(
        open(pack.path(), &NoKeys, &TrustPolicy::AllowUnsigned).unwrap_err(),
        PackError::TocHashMismatch
    );

    let mut header = bytes;
    header[37] = 1; // reserved
    let pack = TempPack::new("hdr", &header);
    assert!(matches!(
        open(pack.path(), &NoKeys, &TrustPolicy::AllowUnsigned).unwrap_err(),
        PackError::BadHeader(_)
    ));
}

#[test]
fn encrypted_pack_needs_the_right_key() {
    let mut spec = plain_spec();
    spec.key = Some(key());
    let pack = TempPack::new("enc", &build(&spec));
    let reader = open(pack.path(), &keys(), &TrustPolicy::AllowUnsigned).unwrap();
    for (key, expected) in &spec.entries {
        assert_eq!(&read(&reader, key).unwrap(), expected);
    }
    assert!(matches!(
        open(pack.path(), &NoKeys, &TrustPolicy::AllowUnsigned).unwrap_err(),
        PackError::KeyUnavailable(_)
    ));
    let (id, generation, _) = key();
    let wrong = StaticKeys::new().with(id, generation, ContentKey::from_bytes([1; 32]));
    assert_eq!(
        open(pack.path(), &wrong, &TrustPolicy::AllowUnsigned).unwrap_err(),
        PackError::TocAuthFailed
    );
    // Names are not readable without the key: the TOC is sealed.
    let bytes = std::fs::read(pack.path()).unwrap();
    assert!(!bytes.windows(11).any(|w| w == b"ui/logo.png"));
}

/// Swap two records that hash-check (re-hashing the TOC entry is not
/// possible for an attacker without the key, but moving whole records
/// inside an extent while keeping hashes is): AEAD associated data rejects
/// the moved block.
#[test]
fn block_moved_between_entries_fails_authentication() {
    let mut spec = plain_spec();
    spec.entries = vec![
        ("a.bin", content(1, BLOCK as usize)),
        ("b.bin", content(2, BLOCK as usize)),
    ];
    spec.key = Some(key());
    let bytes = build(&spec);
    let pack = TempPack::new("swap-probe", &bytes);
    let reader = open(pack.path(), &keys(), &TrustPolicy::AllowUnsigned).unwrap();
    let (a, a_blocks) = reader.stored_entry("a.bin").unwrap();
    let (b, _) = reader.stored_entry("b.bin").unwrap();
    drop(reader);
    let len = a_blocks[0].stored_len as usize;
    let mut swapped = bytes.clone();
    let (ao, bo) = (a.extent_offset as usize, b.extent_offset as usize);
    let record_a = bytes[ao..ao + len].to_vec();
    let record_b = bytes[bo..bo + len].to_vec();
    swapped[ao..ao + len].copy_from_slice(&record_b);
    swapped[bo..bo + len].copy_from_slice(&record_a);
    let pack = TempPack::new("swap", &swapped);
    let reader = open(pack.path(), &keys(), &TrustPolicy::AllowUnsigned).unwrap();
    // Record hashes are per position in the TOC, so the swap is caught at
    // the hash check first; either way, no bytes come back.
    assert!(read(&reader, "a.bin").unwrap_err().is_integrity_failure());
    assert!(read(&reader, "b.bin").unwrap_err().is_integrity_failure());
}

fn ek(key: &[u8]) -> [u8; 16] {
    crate::hash::entry_key_id(key)
}

#[test]
fn aad_binds_entry_index_and_generation() {
    let key = ContentKey::from_bytes([5; 32]);
    let id = crate::hash::pack_id("a", "ui");
    let sealed = seal::seal(
        &key,
        &[1; 24],
        &block_aad(&id, &ek(b"a.bin"), 0, 1, 3, 0),
        b"abc",
    );
    for other in [
        block_aad(&id, &ek(b"b.bin"), 0, 1, 3, 0),
        block_aad(&id, &ek(b"a.bin"), 1, 1, 3, 0),
        block_aad(&id, &ek(b"a.bin"), 0, 2, 3, 0),
        block_aad(&id, &ek(b"a.bin"), 0, 1, 3, 1),
        block_aad(&crate::hash::pack_id("a", "fx"), &ek(b"a.bin"), 0, 1, 3, 0),
    ] {
        assert!(seal::open(&key, &other, &sealed).is_none());
    }
}

#[test]
fn signature_policy_is_enforced() {
    let signer = SigningKey::from_bytes(&[8; 32]);
    let publisher = PublisherKey::from_bytes(signer.verifying_key().as_bytes()).unwrap();
    let trust = TrustPolicy::RequirePublisher(publisher);

    let unsigned = TempPack::new("unsigned", &build(&plain_spec()));
    assert_eq!(
        open(unsigned.path(), &NoKeys, &trust).unwrap_err(),
        PackError::SignatureRequired
    );

    let mut spec = plain_spec();
    spec.signer = Some(&signer);
    let bytes = build(&spec);
    let signed = TempPack::new("signed", &bytes);
    assert!(open(signed.path(), &NoKeys, &trust).is_ok());
    // AllowUnsigned accepts it as well (the signature is not checked).
    assert!(open(signed.path(), &NoKeys, &TrustPolicy::AllowUnsigned).is_ok());

    let other = SigningKey::from_bytes(&[9; 32]);
    let mut other_spec = plain_spec();
    other_spec.signer = Some(&other);
    let foreign = TempPack::new("foreign-signer", &build(&other_spec));
    assert_eq!(
        open(foreign.path(), &NoKeys, &trust).unwrap_err(),
        PackError::UnknownPublisher
    );

    // Any signed header field: here the entry count.
    let mut tampered = bytes;
    tampered[52] ^= 1;
    let tampered = TempPack::new("tampered", &tampered);
    assert_eq!(
        open(tampered.path(), &NoKeys, &trust).unwrap_err(),
        PackError::SignatureInvalid
    );
}

#[test]
fn expected_pack_pins_the_manifest_build() {
    let bytes = build(&plain_spec());
    let pack = TempPack::new("pinned", &bytes);
    let header = Header::decode(&bytes).unwrap();
    let good = ExpectedPack {
        pack_id: header.pack_id,
        toc_hash: header.toc_hash,
        class: ResourceClass::Protected,
        encrypted: false,
        signed: false,
        key_generation: 0,
    };
    assert!(
        PackReader::open(
            pack.path(),
            "ui",
            &NoKeys,
            &TrustPolicy::AllowUnsigned,
            Some(&good)
        )
        .is_ok()
    );
    let stale = ExpectedPack {
        toc_hash: [0; 32],
        ..good
    };
    assert_eq!(
        PackReader::open(
            pack.path(),
            "ui",
            &NoKeys,
            &TrustPolicy::AllowUnsigned,
            Some(&stale)
        )
        .unwrap_err(),
        PackError::UnexpectedPack
    );
    let relabelled = ExpectedPack {
        class: ResourceClass::BootstrapUi,
        ..good
    };
    assert_eq!(
        PackReader::open(
            pack.path(),
            "ui",
            &NoKeys,
            &TrustPolicy::AllowUnsigned,
            Some(&relabelled)
        )
        .unwrap_err(),
        PackError::UnexpectedPack
    );
}

/// Unsigned but encrypted: editing a header field (here the class, which
/// the key provider sees) breaks TOC authentication.
#[test]
fn encrypted_header_fields_are_authenticated_without_a_signature() {
    let mut spec = plain_spec();
    spec.key = Some(key());
    let mut bytes = build(&spec);
    bytes[36] = ResourceClass::BootstrapUi.to_byte();
    let pack = TempPack::new("relabel", &bytes);
    assert_eq!(
        open(pack.path(), &keys(), &TrustPolicy::AllowUnsigned).unwrap_err(),
        PackError::TocAuthFailed
    );
}

#[test]
fn oversized_window_is_rejected() {
    // Frame header claiming a 1 GiB window (Window_Descriptor exponent 20),
    // not single-segment.
    let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0b0000_0000, 20 << 3];
    frame.extend_from_slice(&[1, 0, 0, b'x']);
    let mut out = Vec::new();
    assert!(!super::reader::decompress_for_test(&frame, 1, &mut out));
    assert!(out.capacity() < 1 << 20);
}

#[test]
fn early_splash_cannot_be_encrypted() {
    let mut spec = plain_spec();
    spec.key = Some(key());
    spec.class = ResourceClass::EarlySplash;
    let pack = TempPack::new("splash", &build(&spec));
    assert!(matches!(
        open(pack.path(), &keys(), &TrustPolicy::AllowUnsigned).unwrap_err(),
        PackError::BadHeader(_)
    ));
}

#[test]
fn compressed_blocks_decode_with_bounds() {
    // A zstd frame (raw block, single segment) of "hello hello hello!".
    let plain = b"hello hello hello!".to_vec();
    let frame = raw_zstd_frame(&plain);
    let mut out = Vec::new();
    assert!(super::reader::decompress_for_test(
        &frame,
        plain.len() as u32,
        &mut out
    ));
    assert_eq!(out, plain);
    // Declared length disagrees with the frame: rejected.
    let mut out = Vec::new();
    assert!(!super::reader::decompress_for_test(
        &frame,
        plain.len() as u32 - 1,
        &mut out
    ));
    let mut out = Vec::new();
    assert!(!super::reader::decompress_for_test(
        &frame,
        plain.len() as u32 + 1,
        &mut out
    ));
}

/// Minimal valid zstd frame holding `data` as one raw block (RFC 8878 §3.1).
fn raw_zstd_frame(data: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd];
    // Frame header descriptor: single segment, 1-byte FCS, no checksum.
    frame.push(0b0010_0000);
    frame.push(data.len() as u8);
    // Block header: last block, raw, size.
    let header = 1u32 | ((data.len() as u32) << 3);
    frame.extend_from_slice(&header.to_le_bytes()[..3]);
    frame.extend_from_slice(data);
    frame
}

fn manifest_pack(
    name: &str,
    file: &str,
    class: ResourceClass,
    prefixes: &[&str],
    header: Option<&Header>,
) -> crate::manifest::ManifestPack {
    crate::manifest::ManifestPack {
        name: name.into(),
        file: file.into(),
        class,
        prefixes: prefixes.iter().map(|p| (*p).to_owned()).collect(),
        format_version: FORMAT_VERSION,
        pack_id: crate::to_hex(&header.map_or([0; 16], |h| h.pack_id)),
        toc_hash: crate::to_hex(&header.map_or([0; 32], |h| h.toc_hash)),
        size: 0,
        entries: header.map_or(0, |h| h.entry_count),
        key_name: None,
        key_id: None,
        key_generation: 0,
        signed: header.is_some_and(Header::signed),
        depends_on: vec![],
    }
}

/// Over the cap, an early-splash pack is refused right after its header: the
/// TOC bounds (which padding breaks) are never reached.
#[test]
fn early_splash_pack_over_the_cap_is_refused_after_the_header() {
    let mut spec = plain_spec();
    spec.class = ResourceClass::EarlySplash;
    let mut bytes = build(&spec);
    let small = TempPack::new("early-splash-small", &bytes);
    assert!(open(small.path(), &NoKeys, &TrustPolicy::AllowUnsigned).is_ok());
    let len = EARLY_SPLASH_MAX_PACK_BYTES + 1;
    bytes.resize(len as usize, 0);
    let large = TempPack::new("early-splash-large", &bytes);
    assert_eq!(
        open(large.path(), &NoKeys, &TrustPolicy::AllowUnsigned).unwrap_err(),
        PackError::EarlySplashTooLarge { len }
    );
}

#[test]
fn manifest_routes_by_longest_claiming_prefix() {
    let mut manifest = crate::manifest::tests::sample();
    manifest.resource_packs = vec![
        manifest_pack("ui", "ui.nrpack", ResourceClass::Protected, &["ui/"], None),
        manifest_pack(
            "fonts",
            "f.nrpack",
            ResourceClass::Protected,
            &["ui/fonts/"],
            None,
        ),
        manifest_pack(
            "readme",
            "r.nrpack",
            ResourceClass::Protected,
            &["readme.txt"],
            None,
        ),
    ];
    let route = |path| manifest.route(path).map(|pack| pack.name.as_str());
    assert_eq!(route("ui/app.css"), Some("ui"));
    assert_eq!(route("ui/fonts/a.ttf"), Some("fonts"));
    assert_eq!(route("readme.txt"), Some("readme"));
    assert_eq!(route("readme.txt.bak"), None);
    assert_eq!(route("uix/a.css"), None);
}

#[test]
fn early_splash_paths_route_to_early_splash_packs_only() {
    use crate::manifest::EarlySplashError;

    let mut manifest = crate::manifest::tests::sample();
    manifest.resource_packs = vec![
        manifest_pack(
            "splash",
            "s.nrpack",
            ResourceClass::EarlySplash,
            &["splash/"],
            None,
        ),
        // A longer prefix inside `splash/` wins the route, and its class
        // refuses it.
        manifest_pack(
            "ui",
            "u.nrpack",
            ResourceClass::Protected,
            &["splash/secret/"],
            None,
        ),
    ];
    let pack = |path| {
        manifest
            .early_splash_pack(path)
            .map(|pack| pack.name.as_str())
    };
    assert_eq!(pack("splash/logo.png"), Ok("splash"));
    assert_eq!(
        pack("splash/secret/logo.png"),
        Err(EarlySplashError::WrongClass {
            pack: "ui".into(),
            class: ResourceClass::Protected
        })
    );
    assert_eq!(pack("boot/logo.png"), Err(EarlySplashError::NotFound));
    for invalid in ["", "splash/../ui/a.png", "nana://res/splash/logo.png"] {
        assert_eq!(
            pack(invalid),
            Err(EarlySplashError::InvalidPath),
            "{invalid:?}"
        );
    }
}
