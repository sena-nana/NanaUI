//! The DX12 shader cache format (`wgpu-hal/src/dx12/pipeline_cache.rs`).
//!
//! The module is pure Rust, so it is compiled here directly, on every OS: its
//! own unit tests (`pipeline_cache::tests`) run as part of this test binary,
//! next to the ones below.

// Vendored wgpu-hal source: it follows wgpu's lint settings, not this
// workspace's Clippy configuration.
#[allow(clippy::all)]
#[path = "../../../wgpu/wgpu-hal/src/dx12/pipeline_cache.rs"]
mod pipeline_cache;

use pipeline_cache::{KeyHasher, ShaderBlobCache, ShaderKey};

fn key(name: &str) -> ShaderKey {
    ShaderKey(KeyHasher::new().field(name.as_bytes()).finish())
}

fn cache_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut cache = ShaderBlobCache::default();
    for (name, bytes) in entries {
        cache.insert(key(name), bytes);
    }
    cache.serialize()
}

/// Bytes 8..12 hold the entry count.
fn entry_count(data: &[u8]) -> u32 {
    u32::from_le_bytes(data[8..12].try_into().unwrap())
}

#[test]
fn entries_round_trip_through_bytes() {
    let data = cache_with(&[("vs", b"vertex bytecode"), ("fs", b"fragment")]);
    assert_eq!(&data[..8], b"NDXSHC01");
    assert_eq!(entry_count(&data), 2);
    let mut loaded = ShaderBlobCache::load(&data);
    assert_eq!(&*loaded.get(&key("vs")).unwrap(), b"vertex bytecode");
    assert_eq!(&*loaded.get(&key("fs")).unwrap(), b"fragment");
    assert!(loaded.get(&key("cs")).is_none());
    // Saving again keeps both: they were just used.
    assert_eq!(entry_count(&loaded.serialize()), 2);
}

#[test]
fn every_truncation_is_refused_whole() {
    let data = cache_with(&[("vs", b"vertex bytecode"), ("fs", b"fragment")]);
    for len in 0..data.len() {
        let mut loaded = ShaderBlobCache::load(&data[..len]);
        assert!(
            loaded.get(&key("vs")).is_none() && loaded.get(&key("fs")).is_none(),
            "a cache cut to {len} of {} bytes was used",
            data.len()
        );
    }
}

#[test]
fn a_corrupt_entry_is_refused_and_the_others_kept() {
    let data = cache_with(&[("vs", b"vertex bytecode"), ("fs", b"fragment")]);
    // Flip one byte of each entry's bytecode in turn (the last bytes of the
    // record); the other entry must survive.
    for (name, other, bytes) in [
        ("vs", "fs", &b"vertex bytecode"[..]),
        ("fs", "vs", &b"fragment"[..]),
    ] {
        let at = data
            .windows(bytes.len())
            .position(|window| window == bytes)
            .unwrap();
        let mut corrupt = data.clone();
        corrupt[at] ^= 0x01;
        let mut loaded = ShaderBlobCache::load(&corrupt);
        assert!(loaded.get(&key(name)).is_none(), "corrupt {name} was used");
        assert!(loaded.get(&key(other)).is_some(), "{other} was dropped");
    }
}

#[test]
fn a_bad_magic_is_refused() {
    let mut data = cache_with(&[("vs", b"vertex bytecode")]);
    data[7] = b'2';
    assert!(ShaderBlobCache::load(&data).get(&key("vs")).is_none());
}

#[test]
fn an_unused_entry_survives_eight_saves_and_not_a_ninth() {
    let mut data = cache_with(&[("stale", b"old"), ("used", b"new")]);
    for save in 1..=9 {
        let mut loaded = ShaderBlobCache::load(&data);
        loaded.get(&key("used")).unwrap();
        data = loaded.serialize();
        let stale_kept = ShaderBlobCache::load(&data).get(&key("stale")).is_some();
        assert_eq!(
            stale_kept,
            save <= 8,
            "unused entry after {save} saves without use"
        );
    }
}

#[test]
fn a_used_entry_never_ages_out() {
    let mut data = cache_with(&[("used", b"new")]);
    for _ in 0..32 {
        let mut loaded = ShaderBlobCache::load(&data);
        loaded.get(&key("used")).unwrap();
        data = loaded.serialize();
    }
    assert!(ShaderBlobCache::load(&data).get(&key("used")).is_some());
}

#[test]
fn compiler_inputs_give_distinct_keys() {
    // The DX12 key hashes HLSL, entry point, profile and flags as separate
    // fields; a change to any one is a different key.
    let base = KeyHasher::new()
        .field(b"hlsl")
        .field(b"main")
        .field(b"vs_6_0")
        .field(&[0])
        .finish();
    for changed in [
        KeyHasher::new()
            .field(b"hlsl ")
            .field(b"main")
            .field(b"vs_6_0")
            .field(&[0])
            .finish(),
        KeyHasher::new()
            .field(b"hlsl")
            .field(b"main2")
            .field(b"vs_6_0")
            .field(&[0])
            .finish(),
        KeyHasher::new()
            .field(b"hlsl")
            .field(b"main")
            .field(b"vs_6_6")
            .field(&[0])
            .finish(),
        KeyHasher::new()
            .field(b"hlsl")
            .field(b"main")
            .field(b"vs_6_0")
            .field(&[1])
            .finish(),
    ] {
        assert_ne!(base, changed);
    }
}
