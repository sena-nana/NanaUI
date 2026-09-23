//! BLAKE3 helpers. Every derived id is domain-separated so a value computed
//! for one purpose never collides with another.

/// Content hash of `bytes`.
pub fn content(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// Hash of `parts` under `domain`. Each part is length-prefixed, so
/// `("ab", "c")` and `("a", "bc")` differ.
pub fn derive(domain: &str, parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(domain.len() as u64).to_le_bytes());
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    *hasher.finalize().as_bytes()
}

/// First `N` bytes of [`derive`].
pub fn derive_prefix<const N: usize>(domain: &str, parts: &[&[u8]]) -> [u8; N] {
    let full = derive(domain, parts);
    let mut out = [0u8; N];
    out.copy_from_slice(&full[..N]);
    out
}

/// A pack's id: stable across builds of the same application, distinct
/// between packs and between applications. Bound into every block's AEAD
/// associated data so blocks cannot move between packs.
pub fn pack_id(application_id: &str, pack_name: &str) -> [u8; 16] {
    derive_prefix(
        "nana.nrpack.pack-id.v1",
        &[application_id.as_bytes(), pack_name.as_bytes()],
    )
}

/// Short hash of an entry key, bound into its blocks' associated data.
pub fn entry_key_id(key: &[u8]) -> [u8; 16] {
    derive_prefix("nana.nrpack.entry-key.v1", &[key])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_length_prefixed_and_domain_separated() {
        assert_ne!(derive("d", &[b"ab", b"c"]), derive("d", &[b"a", b"bc"]));
        assert_ne!(derive("d1", &[b"x"]), derive("d2", &[b"x"]));
        assert_ne!(pack_id("app", "ui"), pack_id("app", "bootstrap"));
        assert_ne!(pack_id("app.a", "ui"), pack_id("app.b", "ui"));
        assert_eq!(pack_id("app", "ui"), pack_id("app", "ui"));
    }
}
