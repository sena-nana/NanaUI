//! `.nrpack` resource packs.
//!
//! A pack is a header, a data region of per-entry extents, and a table of
//! contents (TOC). Each entry is split into fixed-size plaintext blocks; each
//! block is compressed and sealed on its own, so a read touches only the
//! blocks of one entry and a changed resource rewrites only its own extent
//! plus the TOC. See [`format`] for the byte layout and `docs/packaging.md`
//! for why it is shaped this way (SteamPipe deltas).

pub mod format;
mod reader;
pub mod seal;

use std::fmt;

pub use reader::{
    EntryContext, EntryInfo, ExpectedPack, PackReader, ReadStats, check_blocks, decode_records,
    stored_bound,
};

/// Startup class of a pack (Issue #226 §7). Ordered: a pack may only depend
/// on packs of the same or an earlier class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceClass {
    /// Logo and minimal splash configuration; readable before the Nana
    /// runtime exists, never encrypted.
    EarlySplash,
    /// What the first real UI (loading / login / error) needs; its key, if
    /// any, must not depend on a flow that UI drives.
    BootstrapUi,
    /// Ordinary application resources.
    Protected,
}

impl ResourceClass {
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::EarlySplash => 0,
            Self::BootstrapUi => 1,
            Self::Protected => 2,
        }
    }

    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::EarlySplash),
            1 => Some(Self::BootstrapUi),
            2 => Some(Self::Protected),
            _ => None,
        }
    }

    /// Name used in `nana-package.toml` and the manifest.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EarlySplash => "early-splash",
            Self::BootstrapUi => "bootstrap-ui",
            Self::Protected => "protected",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "early-splash" => Some(Self::EarlySplash),
            "bootstrap-ui" => Some(Self::BootstrapUi),
            "protected" => Some(Self::Protected),
            _ => None,
        }
    }
}

impl serde::Serialize for ResourceClass {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ResourceClass {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown resource class `{text}` (early-splash | bootstrap-ui | protected)"
            ))
        })
    }
}

/// Why a pack could not be opened or an entry could not be read. Every
/// integrity or authentication failure is an error: nothing unverified is
/// ever returned.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackError {
    Io(String),
    /// The pack file does not exist.
    Missing,
    /// Not an `.nrpack` file (bad magic or header length).
    NotAPack,
    UnsupportedVersion(u16),
    /// Reserved flag bits set, unknown class, or inconsistent header fields.
    BadHeader(&'static str),
    Truncated,
    /// The trust policy requires a signature and the pack has none.
    SignatureRequired,
    /// Signed by a key other than the pinned publisher.
    UnknownPublisher,
    SignatureInvalid,
    /// The pack is not the one the manifest lists (id or TOC hash differ).
    UnexpectedPack,
    TocHashMismatch,
    KeyUnavailable(String),
    TocAuthFailed,
    TocCorrupt(&'static str),
    NotFound,
    TooLarge {
        len: u64,
    },
    BlockHashMismatch {
        block: u32,
    },
    BlockAuthFailed {
        block: u32,
    },
    Decompress {
        block: u32,
    },
    PlaintextHashMismatch,
}

impl PackError {
    /// Stable numeric code, for diagnostics fields and process exit reports.
    pub const fn code(&self) -> u64 {
        match self {
            Self::Io(_) => 1,
            Self::Missing => 2,
            Self::NotAPack => 3,
            Self::UnsupportedVersion(_) => 4,
            Self::BadHeader(_) => 5,
            Self::Truncated => 6,
            Self::SignatureRequired => 7,
            Self::UnknownPublisher => 8,
            Self::SignatureInvalid => 9,
            Self::UnexpectedPack => 10,
            Self::TocHashMismatch => 11,
            Self::KeyUnavailable(_) => 12,
            Self::TocAuthFailed => 13,
            Self::TocCorrupt(_) => 14,
            Self::NotFound => 15,
            Self::TooLarge { .. } => 16,
            Self::BlockHashMismatch { .. } => 17,
            Self::BlockAuthFailed { .. } => 18,
            Self::Decompress { .. } => 19,
            Self::PlaintextHashMismatch => 20,
        }
    }

    /// Integrity / authenticity failures, as opposed to "not there".
    pub const fn is_integrity_failure(&self) -> bool {
        matches!(
            self,
            Self::SignatureInvalid
                | Self::UnknownPublisher
                | Self::UnexpectedPack
                | Self::TocHashMismatch
                | Self::TocAuthFailed
                | Self::TocCorrupt(_)
                | Self::BlockHashMismatch { .. }
                | Self::BlockAuthFailed { .. }
                | Self::Decompress { .. }
                | Self::PlaintextHashMismatch
        )
    }
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Missing => f.write_str("pack file is missing"),
            Self::NotAPack => f.write_str("not an .nrpack file"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported .nrpack version {v}"),
            Self::BadHeader(why) => write!(f, "invalid pack header: {why}"),
            Self::Truncated => f.write_str("pack is truncated"),
            Self::SignatureRequired => {
                f.write_str("pack is not signed and a signature is required")
            }
            Self::UnknownPublisher => f.write_str("pack is signed by an unknown publisher"),
            Self::SignatureInvalid => f.write_str("pack signature is invalid"),
            Self::UnexpectedPack => f.write_str("pack does not match the package manifest"),
            Self::TocHashMismatch => f.write_str("pack TOC hash mismatch"),
            Self::KeyUnavailable(why) => write!(f, "content key unavailable: {why}"),
            Self::TocAuthFailed => f.write_str("pack TOC failed authentication"),
            Self::TocCorrupt(why) => write!(f, "pack TOC is corrupt: {why}"),
            Self::NotFound => f.write_str("entry not found"),
            Self::TooLarge { len } => write!(f, "entry is too large ({len} bytes)"),
            Self::BlockHashMismatch { block } => write!(f, "block {block} hash mismatch"),
            Self::BlockAuthFailed { block } => write!(f, "block {block} failed authentication"),
            Self::Decompress { block } => write!(f, "block {block} failed to decompress"),
            Self::PlaintextHashMismatch => f.write_str("entry content hash mismatch"),
        }
    }
}

impl std::error::Error for PackError {}

#[cfg(test)]
mod tests;
