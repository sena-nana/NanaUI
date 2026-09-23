use super::*;
use nana_package::pack::ExpectedPack;

pub(crate) struct Scratch(pub PathBuf);

impl Scratch {
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("nana-packager-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn content(seed: u32, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    (0..len)
        .map(|i| {
            if i % 3 == 0 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
            }
            (state >> (i % 3 * 8)) as u8 & if seed.is_multiple_of(2) { 0x0f } else { 0xff }
        })
        .collect()
}

fn entries(changes: &[(&str, Vec<u8>)]) -> Vec<InputEntry> {
    let mut map: BTreeMap<String, Vec<u8>> = (0..40)
        .map(|i| {
            (
                format!("ui/{i:03}.bin"),
                content(i, 3_000 + i as usize * 1_700),
            )
        })
        .collect();
    map.insert("fonts/a.ttf".into(), content(99, 200_000));
    map.insert("ui/empty".into(), Vec::new());
    for (key, bytes) in changes {
        if bytes.is_empty() && key.starts_with('-') {
            map.remove(&key[1..]);
        } else {
            map.insert((*key).to_owned(), bytes.clone());
        }
    }
    map.into_iter()
        .map(|(key, bytes)| InputEntry {
            key,
            source: EntrySource::Bytes(bytes),
        })
        .collect()
}

fn key() -> (KeyId, u32, ContentKey) {
    (
        KeyId::from_name("main"),
        1,
        ContentKey::from_bytes([0x33; 32]),
    )
}

fn input<'a>(
    entries: Vec<InputEntry>,
    baseline: Option<&'a Path>,
    cache: Option<&'a ArtifactCache>,
    encryption: Option<(KeyId, u32, ContentKey)>,
    signer: Option<&'a SigningKey>,
) -> PackInput<'a> {
    PackInput {
        application_id: "dev.nana.test",
        name: "ui",
        class: ResourceClass::Protected,
        entries,
        block_size: 16 * 1024,
        zstd_level: Some(3),
        encryption,
        signer,
        baseline,
        cache,
        compact: false,
        max_free_ratio: 0.25,
    }
}

fn read_all(
    path: &Path,
    encryption: Option<&(KeyId, u32, ContentKey)>,
    trust: &TrustPolicy,
    expected: &[InputEntry],
) {
    let keys = match encryption {
        Some((id, generation, key)) => StaticKeys::new().with(*id, *generation, key.clone()),
        None => StaticKeys::new(),
    };
    let reader = PackReader::open(path, "ui", &keys, trust, None).unwrap();
    for entry in expected {
        let EntrySource::Bytes(bytes) = &entry.source else {
            unreachable!()
        };
        assert_eq!(
            &reader
                .read(&entry.key, u64::MAX, &mut ReadStats::default())
                .unwrap(),
            bytes,
            "{}",
            entry.key
        );
    }
    assert_eq!(reader.entry_count() as usize, expected.len());
}

#[test]
fn built_pack_round_trips_plain_encrypted_and_signed() {
    let scratch = Scratch::new("roundtrip");
    let signer = SigningKey::from_bytes(&[7; 32]);
    let publisher = PublisherKey::from_bytes(signer.verifying_key().as_bytes()).unwrap();
    let path = scratch.0.join("ui.nrpack");
    let built = build_pack(
        input(entries(&[]), None, None, Some(key()), Some(&signer)),
        &path,
    )
    .unwrap();
    assert!(built.header.encrypted() && built.header.signed());
    read_all(
        &path,
        Some(&key()),
        &TrustPolicy::RequirePublisher(publisher),
        &entries(&[]),
    );
    // Compressible entries were compressed.
    assert!(built.report.pack_bytes < built.report.source_bytes);
    let expected = ExpectedPack {
        pack_id: built.header.pack_id,
        toc_hash: built.header.toc_hash,
        class: ResourceClass::Protected,
        encrypted: true,
        signed: true,
        key_generation: 1,
    };
    let keys = StaticKeys::new().with(key().0, 1, key().2);
    assert!(
        PackReader::open(
            &path,
            "ui",
            &keys,
            &TrustPolicy::AllowUnsigned,
            Some(&expected)
        )
        .is_ok()
    );

    let plain = scratch.0.join("plain.nrpack");
    build_pack(input(entries(&[]), None, None, None, None), &plain).unwrap();
    read_all(&plain, None, &TrustPolicy::AllowUnsigned, &entries(&[]));
}

#[test]
fn unchanged_rebuild_against_baseline_is_byte_identical() {
    let scratch = Scratch::new("s0");
    let signer = SigningKey::from_bytes(&[7; 32]);
    let first = scratch.0.join("v1.nrpack");
    build_pack(
        input(entries(&[]), None, None, Some(key()), Some(&signer)),
        &first,
    )
    .unwrap();
    let second = scratch.0.join("v2.nrpack");
    let built = build_pack(
        input(entries(&[]), Some(&first), None, Some(key()), Some(&signer)),
        &second,
    )
    .unwrap();
    assert_eq!(built.report.encoded_bytes, 0);
    assert!(built.report.toc_reused);
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
}

#[test]
fn cache_alone_reproduces_an_unchanged_build() {
    let scratch = Scratch::new("cache");
    let cache = ArtifactCache::new(&scratch.0.join("cache"));
    let first = scratch.0.join("v1.nrpack");
    build_pack(
        input(entries(&[]), None, Some(&cache), Some(key()), None),
        &first,
    )
    .unwrap();
    let second = scratch.0.join("v2.nrpack");
    let built = build_pack(
        input(entries(&[]), None, Some(&cache), Some(key()), None),
        &second,
    )
    .unwrap();
    assert_eq!(built.report.encoded_bytes, 0);
    assert_eq!(built.report.cache_hits, 42);
    // Records are identical; only the sealed TOC differs (fresh nonce).
    let (a, b) = (
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap(),
    );
    let toc = built.header.toc_offset as usize;
    assert_eq!(a[HEADER_LEN..toc], b[HEADER_LEN..toc]);
}

#[test]
fn poisoned_cache_entries_are_re_encoded() {
    let scratch = Scratch::new("poison");
    let cache_dir = scratch.0.join("cache");
    let cache = ArtifactCache::new(&cache_dir);
    let first = scratch.0.join("v1.nrpack");
    build_pack(
        input(entries(&[]), None, Some(&cache), Some(key()), None),
        &first,
    )
    .unwrap();
    // Flip one byte in every cached record file.
    let mut stack = vec![cache_dir.clone()];
    while let Some(dir) = stack.pop() {
        for item in std::fs::read_dir(dir).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let mut bytes = std::fs::read(&path).unwrap();
                let last = bytes.len() - 1;
                bytes[last] ^= 0x80;
                std::fs::write(&path, bytes).unwrap();
            }
        }
    }
    let second = scratch.0.join("v2.nrpack");
    let built = build_pack(
        input(entries(&[]), None, Some(&cache), Some(key()), None),
        &second,
    )
    .unwrap();
    assert_eq!(built.report.cache_hits, 0);
    assert_eq!(built.report.cache_misses, 42);
    read_all(
        &second,
        Some(&key()),
        &TrustPolicy::AllowUnsigned,
        &entries(&[]),
    );
}

#[test]
fn key_rotation_is_reported() {
    let scratch = Scratch::new("rotate");
    let first = scratch.0.join("v1.nrpack");
    build_pack(input(entries(&[]), None, None, Some(key()), None), &first).unwrap();
    let rotated = (
        KeyId::from_name("main"),
        2,
        ContentKey::from_bytes([0x44; 32]),
    );
    let second = scratch.0.join("v2.nrpack");
    let built = build_pack(
        input(
            entries(&[]),
            Some(&first),
            None,
            Some(rotated.clone()),
            None,
        ),
        &second,
    )
    .unwrap();
    assert!(built.report.key_rotation);
    assert_eq!(built.report.reused_bytes, 0);
    read_all(
        &second,
        Some(&rotated),
        &TrustPolicy::AllowUnsigned,
        &entries(&[]),
    );
}

#[test]
fn changed_entry_is_rewritten_in_place_and_others_keep_offsets() {
    let scratch = Scratch::new("inplace");
    let first = scratch.0.join("v1.nrpack");
    build_pack(input(entries(&[]), None, None, Some(key()), None), &first).unwrap();
    let smaller = content(5, 1_000);
    let changes = [("ui/010.bin", smaller)];
    let second = scratch.0.join("v2.nrpack");
    let built = build_pack(
        input(entries(&changes), Some(&first), None, Some(key()), None),
        &second,
    )
    .unwrap();
    assert_eq!(built.report.rewritten_in_place, 1);
    assert_eq!(built.report.kept_in_place, 41);
    assert_eq!(built.report.relocated + built.report.added, 0);
    read_all(
        &second,
        Some(&key()),
        &TrustPolicy::AllowUnsigned,
        &entries(&changes),
    );

    // Growing past the capacity relocates; removing frees space.
    let grown = [
        ("ui/010.bin", content(5, 60_000)),
        ("-ui/020.bin", Vec::new()),
    ];
    let third = scratch.0.join("v3.nrpack");
    let built = build_pack(
        input(entries(&grown), Some(&second), None, Some(key()), None),
        &third,
    )
    .unwrap();
    assert_eq!(built.report.relocated, 1);
    assert_eq!(built.report.removed, 1);
    read_all(
        &third,
        Some(&key()),
        &TrustPolicy::AllowUnsigned,
        &entries(&grown),
    );
}

#[test]
fn excessive_free_space_compacts() {
    let scratch = Scratch::new("compact");
    let first = scratch.0.join("v1.nrpack");
    build_pack(input(entries(&[]), None, None, None, None), &first).unwrap();
    // Remove most entries: the free ratio goes far past 25 %.
    let removed: Vec<(String, Vec<u8>)> = (0..35)
        .map(|i| (format!("-ui/{i:03}.bin"), Vec::new()))
        .collect();
    let removed: Vec<(&str, Vec<u8>)> = removed
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    let second = scratch.0.join("v2.nrpack");
    let built = build_pack(
        input(entries(&removed), Some(&first), None, None, None),
        &second,
    )
    .unwrap();
    assert!(built.report.compacted);
    assert!(
        built
            .report
            .warnings
            .iter()
            .any(|w| w.contains("compacting"))
    );
    read_all(
        &second,
        None,
        &TrustPolicy::AllowUnsigned,
        &entries(&removed),
    );
}

/// Random edit sequences, each rebuilt against the previous pack: every
/// entry reads back, and no two extents overlap.
#[test]
fn random_edit_sequences_keep_packs_consistent() {
    let scratch = Scratch::new("random");
    let mut state = 0x1234_5678_9abc_def0u64;
    let mut next = move |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut previous: Option<PathBuf> = None;
    for round in 0..40 {
        for _ in 0..1 + next(4) {
            let name = format!("r/{:02}.bin", next(24));
            match next(4) {
                0 => {
                    files.remove(&name);
                }
                _ => {
                    let len = next(3) as usize * next(40_000) as usize;
                    let seed = next(1 << 30) as u32;
                    files.insert(name, content(seed, len));
                }
            }
        }
        if files.is_empty() {
            continue;
        }
        let entries: Vec<InputEntry> = files
            .iter()
            .map(|(key, bytes)| InputEntry {
                key: key.clone(),
                source: EntrySource::Bytes(bytes.clone()),
            })
            .collect();
        let out = scratch.0.join(format!("v{round}.nrpack"));
        let mut pack = input(
            entries.clone(),
            previous.as_deref(),
            None,
            Some(key()),
            None,
        );
        pack.max_free_ratio = 0.5;
        build_pack(pack, &out).unwrap();
        read_all(&out, Some(&key()), &TrustPolicy::AllowUnsigned, &entries);

        let (id, generation, content_key) = key();
        let keys = StaticKeys::new().with(id, generation, content_key);
        let reader =
            PackReader::open(&out, "ui", &keys, &TrustPolicy::AllowUnsigned, None).unwrap();
        let mut extents: Vec<(u64, u64)> = reader
            .keys()
            .unwrap()
            .iter()
            .map(|key| reader.stored_entry(key).unwrap().0)
            .filter(|entry| entry.extent_capacity > 0)
            .map(|entry| {
                (
                    entry.extent_offset,
                    entry.extent_offset + entry.extent_capacity,
                )
            })
            .collect();
        extents.sort_unstable();
        for pair in extents.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "round {round}: overlapping extents {pair:?}"
            );
        }
        previous = Some(out);
    }
}

/// Flip random bytes of an unsigned, unencrypted pack (the weakest
/// configuration): opening and reading never panic, and a read returns
/// either the original bytes or an error, never different content.
#[test]
fn corrupted_bytes_never_yield_wrong_content() {
    let scratch = Scratch::new("corrupt");
    let path = scratch.0.join("base.nrpack");
    let originals = entries(&[]);
    build_pack(input(originals.clone(), None, None, None, None), &path).unwrap();
    let pristine = std::fs::read(&path).unwrap();
    let mut state = 0x0bad_5eed_u64;
    let mut next = move |bound: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % bound as u64) as usize
    };
    let target = scratch.0.join("mutated.nrpack");
    for _ in 0..400 {
        let mut bytes = pristine.clone();
        for _ in 0..1 + next(3) {
            let at = next(bytes.len());
            bytes[at] ^= 1 << next(8);
        }
        std::fs::write(&target, &bytes).unwrap();
        let Ok(reader) =
            PackReader::open(&target, "ui", &NoKeys, &TrustPolicy::AllowUnsigned, None)
        else {
            continue;
        };
        for entry in &originals {
            let EntrySource::Bytes(expected) = &entry.source else {
                unreachable!()
            };
            if let Ok(bytes) = reader.read(&entry.key, 16 << 20, &mut ReadStats::default()) {
                assert_eq!(&bytes, expected, "{} read back different bytes", entry.key);
            }
        }
    }
}
