//! Runtime side of the Nana application packaging contract (Issue #226).
//!
//! - [`manifest`]: the machine-readable package manifest written next to a
//!   packaged application (`<RuntimeManifest>/package.json`).
//! - [`pack`]: the `.nrpack` resource pack format and its random-access,
//!   fail-closed reader.
//! - [`keys`]: content keys ([`KeyProvider`]) and publisher trust
//!   ([`TrustPolicy`]).
//! - [`identity`]: the application identity marker a binary embeds through
//!   `nana_ui_platform::application_identity!`.
//!
//! Everything here reads. Building packs, generating keys and signing is the
//! build-side `nana-packager`, which never ships inside an application. See
//! `docs/packaging.md` for the byte layouts and the security model.

pub mod hash;
pub mod identity;
pub mod keys;
pub mod manifest;
pub mod pack;

pub use identity::{EmbeddedIdentity, MarkerError, find_marker};
pub use keys::{
    ContentKey, KeyError, KeyId, KeyProvider, KeyRequest, NoKeys, PublisherKey, PublisherKeyId,
    StaticKeys, TrustPolicy,
};
pub use manifest::{
    MANIFEST_FILE, MANIFEST_SCHEMA, MANIFEST_SCHEMA_VERSION, MANIFEST_SIGNATURE_FILE,
    ManifestError, PackageManifest, SignatureState,
};
pub use pack::{EntryInfo, ExpectedPack, PackError, PackReader, ReadStats, ResourceClass};

/// Version of the Nana packaging contract this crate implements, recorded in
/// every manifest.
pub const NANA_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Environment variable the validator sets to turn a packaged
/// application's startup into a read-only self-check.
pub const SELF_CHECK_ENV: &str = "NANA_PACKAGE_VALIDATE";
/// Prefix of the one JSON line the self-check prints.
pub const SELF_CHECK_PREFIX: &str = "{\"nana_package_validate\":";

/// A logical path inside a package: non-empty `/`-separated segments, none
/// `.` or `..`, no backslash, colon (`C:/x` is absolute on Windows) or
/// control character, at most [`pack::format::MAX_KEY_LEN`] bytes. Resource
/// keys, manifest paths and pack prefixes all follow it.
pub fn valid_logical_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= pack::format::MAX_KEY_LEN
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// Lowercase hex, used for ids and hashes in text formats.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Parse lowercase or uppercase hex into exactly `N` bytes.
pub fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.as_bytes();
    if text.len() != N * 2 {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = (nibble(text[2 * i])? << 4) | nibble(text[2 * i + 1])?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_bad_input() {
        let bytes = [0x00, 0x7f, 0xab, 0xff];
        assert_eq!(to_hex(&bytes), "007fabff");
        assert_eq!(from_hex::<4>("007FABff"), Some(bytes));
        assert_eq!(from_hex::<4>("007fab"), None);
        assert_eq!(from_hex::<4>("007fabfg"), None);
    }
}
