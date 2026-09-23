//! Content keys and publisher trust.
//!
//! Two separate key kinds, never interchangeable:
//!
//! - **Content keys** (XChaCha20-Poly1305, 32 bytes) encrypt and authenticate
//!   pack blocks and TOCs. The application obtains them at run time through a
//!   [`KeyProvider`]; how (embedded, from a license service, from Steam) is
//!   application / distribution policy, not this crate's.
//! - **Publisher keys** (Ed25519) sign pack headers and the package manifest.
//!   Only the public half ever reaches an application, pinned through
//!   [`TrustPolicy::RequirePublisher`].

use std::collections::BTreeMap;
use std::fmt;

use ed25519_dalek::{Signature, VerifyingKey};
use zeroize::Zeroizing;

use crate::hash;
use crate::pack::ResourceClass;

/// Names a content key without revealing it: the first 8 bytes of a
/// domain-separated hash of the key's name in `nana-package.toml`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(pub [u8; 8]);

impl KeyId {
    pub const NONE: Self = Self([0; 8]);

    pub fn from_name(name: &str) -> Self {
        Self(hash::derive_prefix(
            "nana.content-key-id.v1",
            &[name.as_bytes()],
        ))
    }

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }
}

impl fmt::Debug for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyId({})", crate::to_hex(&self.0))
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::to_hex(&self.0))
    }
}

/// A 256-bit content key. Zeroed on drop; `Debug` never prints it.
#[derive(Clone)]
pub struct ContentKey(Zeroizing<[u8; 32]>);

impl ContentKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// 64 hex digits, the form keys take in environment variables and files.
    pub fn from_hex(text: &str) -> Option<Self> {
        crate::from_hex::<32>(text.trim()).map(Self::from_bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ContentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ContentKey(<redacted>)")
    }
}

/// What a pack asks its [`KeyProvider`] for.
#[derive(Debug, Clone, Copy)]
pub struct KeyRequest<'a> {
    /// Pack name as listed in the manifest (or the file stem).
    pub pack: &'a str,
    pub key_id: KeyId,
    pub generation: u32,
    /// Startup class of the pack, so a provider can refuse to block
    /// `BootstrapUI` on a network round trip.
    pub class: ResourceClass,
}

/// The provider cannot supply the key now (it does not hold it, or a
/// license / network source has not delivered it yet). Lookups retry later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyError;

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("content key unavailable")
    }
}

/// Supplies content keys to pack readers. Called once per pack mount, never
/// per read or per frame.
pub trait KeyProvider: Send + Sync {
    fn content_key(&self, request: &KeyRequest<'_>) -> Result<ContentKey, KeyError>;
}

/// No keys: encrypted packs fail with `KeyUnavailable`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoKeys;

impl KeyProvider for NoKeys {
    fn content_key(&self, _: &KeyRequest<'_>) -> Result<ContentKey, KeyError> {
        Err(KeyError)
    }
}

/// A fixed table of keys, for applications that ship their key in the
/// binary and for tests. Shipping a key in the binary only stops casual
/// extraction; see `docs/packaging.md`.
#[derive(Debug, Default, Clone)]
pub struct StaticKeys(BTreeMap<(KeyId, u32), ContentKey>);

impl StaticKeys {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key_id: KeyId, generation: u32, key: ContentKey) -> Self {
        self.insert(key_id, generation, key);
        self
    }

    pub fn insert(&mut self, key_id: KeyId, generation: u32, key: ContentKey) {
        self.0.insert((key_id, generation), key);
    }
}

impl KeyProvider for StaticKeys {
    fn content_key(&self, request: &KeyRequest<'_>) -> Result<ContentKey, KeyError> {
        self.0
            .get(&(request.key_id, request.generation))
            .cloned()
            .ok_or(KeyError)
    }
}

/// First 8 bytes of a hash of an Ed25519 public key; recorded in pack
/// headers and manifests so a reader can tell "signed by someone else" from
/// "corrupted".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublisherKeyId(pub [u8; 8]);

impl fmt::Debug for PublisherKeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublisherKeyId({})", crate::to_hex(&self.0))
    }
}

impl fmt::Display for PublisherKeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::to_hex(&self.0))
    }
}

/// A pinned Ed25519 publisher public key.
#[derive(Clone, PartialEq, Eq)]
pub struct PublisherKey {
    key: VerifyingKey,
}

impl fmt::Debug for PublisherKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublisherKey({})", self.to_text())
    }
}

impl PublisherKey {
    pub const TEXT_PREFIX: &'static str = "ed25519:";

    pub fn from_bytes(bytes: &[u8; 32]) -> Option<Self> {
        VerifyingKey::from_bytes(bytes).ok().map(|key| Self { key })
    }

    /// `ed25519:<64 hex digits>`.
    pub fn from_text(text: &str) -> Option<Self> {
        let hex = text.trim().strip_prefix(Self::TEXT_PREFIX)?;
        Self::from_bytes(&crate::from_hex::<32>(hex)?)
    }

    pub fn to_text(&self) -> String {
        format!(
            "{}{}",
            Self::TEXT_PREFIX,
            crate::to_hex(self.key.as_bytes())
        )
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        self.key.as_bytes()
    }

    pub fn id(&self) -> PublisherKeyId {
        PublisherKeyId(hash::derive_prefix(
            "nana.publisher-key-id.v1",
            &[self.key.as_bytes()],
        ))
    }

    /// Strict Ed25519 verification (rejects malleable and small-order
    /// encodings).
    pub fn verify(&self, message: &[u8], signature: &[u8; 64]) -> bool {
        self.key
            .verify_strict(message, &Signature::from_bytes(signature))
            .is_ok()
    }
}

/// Whether a reader demands a publisher signature.
#[derive(Debug, Clone)]
pub enum TrustPolicy {
    /// Unsigned packs and manifests are accepted; a signature that is present
    /// is not checked (there is nothing to check it against). Integrity
    /// hashes are still verified. Development and tests.
    AllowUnsigned,
    /// Every pack and the manifest must carry a valid signature by this key.
    RequirePublisher(PublisherKey),
}

impl TrustPolicy {
    pub fn publisher(&self) -> Option<&PublisherKey> {
        match self {
            Self::AllowUnsigned => None,
            Self::RequirePublisher(key) => Some(key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_key_debug_is_redacted() {
        let key = ContentKey::from_bytes([7; 32]);
        assert_eq!(format!("{key:?}"), "ContentKey(<redacted>)");
        assert!(ContentKey::from_hex(&"07".repeat(32)).is_some());
        assert!(ContentKey::from_hex("07").is_none());
    }

    #[test]
    fn key_id_is_stable_and_nonzero() {
        assert_eq!(KeyId::from_name("main"), KeyId::from_name("main"));
        assert_ne!(KeyId::from_name("main"), KeyId::from_name("dlc"));
        assert!(!KeyId::from_name("main").is_none());
    }

    #[test]
    fn publisher_key_text_round_trips() {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[3; 32]);
        let public = PublisherKey::from_bytes(signing.verifying_key().as_bytes()).unwrap();
        let parsed = PublisherKey::from_text(&public.to_text()).unwrap();
        assert_eq!(parsed, public);
        assert!(PublisherKey::from_text("rsa:00").is_none());
    }

    #[test]
    fn static_keys_match_id_and_generation() {
        let id = KeyId::from_name("main");
        let keys = StaticKeys::new().with(id, 2, ContentKey::from_bytes([1; 32]));
        let request = |generation| KeyRequest {
            pack: "ui",
            key_id: id,
            generation,
            class: ResourceClass::Protected,
        };
        assert!(keys.content_key(&request(2)).is_ok());
        assert_eq!(keys.content_key(&request(1)).unwrap_err(), KeyError);
    }
}
