//! The Early Splash logo read (Issues #225 / #226): the one resource an
//! application reads before the Nana runtime exists.
//!
//! It is as narrow as the package format allows. The manifest — already read
//! and checked against the trust policy — routes the logical path to exactly
//! one pack. That pack has to be an `early-splash` pack, so it is never
//! encrypted and no key provider is asked; a path that routes anywhere else
//! is refused before its pack is opened. The pack opens pinned to the
//! manifest and verified as the trust policy demands, and the entry's length
//! is checked against the caller's cap before any of its data is read.
//! Nothing else in the package is opened, listed or scanned.

use std::fmt;
use std::path::Path;

use crate::keys::{NoKeys, TrustPolicy};
use crate::manifest::PackageManifest;
use crate::pack::{PackError, PackReader, ReadStats, ResourceClass};

/// Why an Early Splash resource could not be read. Nothing unverified is
/// ever returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EarlySplashError {
    /// Not a logical package path.
    InvalidPath,
    /// No pack claims the path, or its pack holds no such entry.
    NotFound,
    /// The path belongs to a pack of another class. That pack was not
    /// opened: only `early-splash` packs are readable before the runtime and
    /// its keys exist.
    WrongClass { pack: String, class: ResourceClass },
    /// Larger than the caller's cap. Only the pack's header and TOC were read.
    TooLarge { bytes: u64 },
    /// The pack could not be opened, or the entry failed verification.
    Pack { pack: String, error: PackError },
}

impl fmt::Display for EarlySplashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath => f.write_str("not a logical package path"),
            Self::NotFound => f.write_str("no early-splash pack holds this path"),
            Self::WrongClass { pack, class } => write!(
                f,
                "the path belongs to pack `{pack}` of class {}; only early-splash packs are read \
                 before the runtime exists",
                class.as_str()
            ),
            Self::TooLarge { bytes } => write!(f, "entry is too large ({bytes} bytes)"),
            Self::Pack { pack, error } => write!(f, "pack `{pack}`: {error}"),
        }
    }
}

impl std::error::Error for EarlySplashError {}

/// Read `path` from the `early-splash` pack `manifest` routes it to.
/// `resources` is the package's RuntimeResources directory; `trust` is the
/// policy the manifest was read with. At most `max_bytes` of plaintext are
/// returned.
pub fn read_early_splash(
    manifest: &PackageManifest,
    resources: &Path,
    path: &str,
    trust: &TrustPolicy,
    max_bytes: u64,
    stats: &mut ReadStats,
) -> Result<Vec<u8>, EarlySplashError> {
    if !crate::valid_logical_path(path) {
        return Err(EarlySplashError::InvalidPath);
    }
    let pack = manifest.route(path).ok_or(EarlySplashError::NotFound)?;
    if pack.class != ResourceClass::EarlySplash {
        return Err(EarlySplashError::WrongClass {
            pack: pack.name.clone(),
            class: pack.class,
        });
    }
    let failed = |error| EarlySplashError::Pack {
        pack: pack.name.clone(),
        error,
    };
    // `NoKeys`: the pin says unencrypted and the reader refuses an encrypted
    // early-splash header, so no key provider, and no key source behind it,
    // is ever consulted here.
    let reader = PackReader::open(
        &resources.join(&pack.file),
        &pack.name,
        &NoKeys,
        trust,
        pack.expected().as_ref(),
    )
    .map_err(failed)?;
    reader
        .read(path, max_bytes, stats)
        .map_err(|error| match error {
            PackError::NotFound => EarlySplashError::NotFound,
            PackError::TooLarge { len } => EarlySplashError::TooLarge { bytes: len },
            error => failed(error),
        })
}
