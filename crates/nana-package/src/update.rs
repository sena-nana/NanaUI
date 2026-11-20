//! Update manifests and the update journal (Issue #274 §5).
//!
//! Nana ships no updater and no update UI. This module is the contract an
//! application's own updater (or a store's) builds on:
//!
//! - [`UpdateManifest`]: what a release offers — target application,
//!   channel, version, the installer artifacts with their sizes and BLAKE3
//!   hashes — signed by the publisher key that signs package manifests.
//!   [`UpdateManifest::verify`] accepts one only if the signature checks,
//!   it is for this application, target and channel, and it is strictly
//!   newer than what is installed: a validly signed older release is never
//!   offered as an update (downgrade replay).
//! - [`UpdateJournal`]: the rollback policy. The updater records the
//!   previous and pending versions before it swaps files; each launch of the
//!   pending version counts as unconfirmed until the application reaches a
//!   healthy point (`UiReady`) and confirms it. A pending version that fails
//!   to confirm `max_unconfirmed_launches` times in a row is rolled back by
//!   the application's own restore hook. The journal is plain data: where it
//!   is stored and how files are restored belong to the application.

use std::cmp::Ordering;
use std::fmt;
use std::io::Read;

use serde::{Deserialize, Serialize};

use crate::keys::PublisherKey;
use crate::manifest::{ManifestTarget, PackageManifest};

pub const UPDATE_FILE: &str = "update.json";
pub const UPDATE_SIGNATURE_FILE: &str = "update.json.sig";
pub const UPDATE_SCHEMA: &str = "nana.update-manifest";
pub const UPDATE_SCHEMA_VERSION: u32 = 1;
pub const UPDATE_SIGNATURE_CONTEXT: &[u8] = b"nana.update-manifest.v1\0";
/// Larger update manifests are refused before their signature is checked.
pub const MAX_UPDATE_BYTES: u64 = 1024 * 1024;

pub const JOURNAL_SCHEMA: &str = "nana.update-journal";
pub const JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateManifest {
    pub schema: String,
    pub schema_version: u32,
    pub application: UpdateApplication,
    /// Release channel (`stable`, `beta`...). An updater only accepts its
    /// own channel.
    pub channel: String,
    pub target: ManifestTarget,
    /// Oldest installed version this update may be applied over; any older
    /// version when absent.
    pub minimum_from_version: Option<String>,
    /// BLAKE3 of the `package.json` the update installs, so the installed
    /// result can be checked against what was announced.
    pub package_manifest_blake3: String,
    pub artifacts: Vec<UpdateArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateApplication {
    pub id: String,
    pub version: String,
    pub build_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateArtifact {
    /// File name, without directories: where it is downloaded from is the
    /// updater's business.
    pub file: String,
    pub format: ArtifactFormat,
    pub size: u64,
    pub blake3: String,
}

/// The installer formats `nana-packager` builds, plus a plain archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactFormat {
    Dmg,
    Msi,
    Nsis,
    Deb,
    Appimage,
    Flatpak,
    Archive,
}

impl ArtifactFormat {
    pub const ALL: [Self; 7] = [
        Self::Dmg,
        Self::Msi,
        Self::Nsis,
        Self::Deb,
        Self::Appimage,
        Self::Flatpak,
        Self::Archive,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dmg => "dmg",
            Self::Msi => "msi",
            Self::Nsis => "nsis",
            Self::Deb => "deb",
            Self::Appimage => "appimage",
            Self::Flatpak => "flatpak",
            Self::Archive => "archive",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.as_str() == text)
    }
}

/// What an updater accepts.
#[derive(Debug, Clone)]
pub struct UpdatePolicy {
    pub channel: String,
    /// Updates are always signed: there is no unsigned policy.
    pub publisher: PublisherKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    TooLarge,
    SignatureInvalid,
    Parse(String),
    UnsupportedSchema {
        schema: String,
        version: u32,
    },
    WrongApplication {
        expected: String,
        found: String,
    },
    WrongTarget {
        expected: String,
        found: String,
    },
    WrongChannel {
        expected: String,
        found: String,
    },
    /// The offered version is not newer than the installed one.
    NotNewer {
        installed: String,
        offered: String,
    },
    /// The installed version is older than `minimum_from_version`.
    InstalledTooOld {
        installed: String,
        minimum: String,
    },
    UnknownArtifact(String),
    ArtifactMismatch(String),
    Io(String),
}

impl UpdateError {
    pub const fn code(&self) -> u64 {
        match self {
            Self::TooLarge => 1,
            Self::SignatureInvalid => 2,
            Self::Parse(_) => 3,
            Self::UnsupportedSchema { .. } => 4,
            Self::WrongApplication { .. } => 5,
            Self::WrongTarget { .. } => 6,
            Self::WrongChannel { .. } => 7,
            Self::NotNewer { .. } => 8,
            Self::InstalledTooOld { .. } => 9,
            Self::UnknownArtifact(_) => 10,
            Self::ArtifactMismatch(_) => 11,
            Self::Io(_) => 12,
        }
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("update manifest is too large"),
            Self::SignatureInvalid => f.write_str("update manifest signature is invalid"),
            Self::Parse(error) => write!(f, "invalid update manifest: {error}"),
            Self::UnsupportedSchema { schema, version } => {
                write!(f, "unsupported update manifest schema {schema} v{version}")
            }
            Self::WrongApplication { expected, found } => {
                write!(f, "update is for `{found}`, not `{expected}`")
            }
            Self::WrongTarget { expected, found } => {
                write!(f, "update targets `{found}`, not `{expected}`")
            }
            Self::WrongChannel { expected, found } => {
                write!(f, "update is on channel `{found}`, not `{expected}`")
            }
            Self::NotNewer { installed, offered } => {
                write!(
                    f,
                    "update {offered} is not newer than the installed {installed}"
                )
            }
            Self::InstalledTooOld { installed, minimum } => write!(
                f,
                "update requires at least {minimum} installed; {installed} is installed"
            ),
            Self::UnknownArtifact(file) => write!(f, "`{file}` is not an artifact of the update"),
            Self::ArtifactMismatch(file) => {
                write!(f, "`{file}` does not match the update manifest")
            }
            Self::Io(error) => write!(f, "cannot read update artifact: {error}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl UpdateManifest {
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("update types always serialize");
        text.push('\n');
        text
    }

    /// The message a publisher signs for `manifest_bytes`. Domain-separated
    /// from package manifests, so neither signature verifies as the other.
    pub fn signing_message(manifest_bytes: &[u8]) -> Vec<u8> {
        let mut message = UPDATE_SIGNATURE_CONTEXT.to_vec();
        message.extend_from_slice(manifest_bytes);
        message
    }

    /// Parse without checking a signature, for tools that inspect what they
    /// built. An updater uses [`verify`](Self::verify).
    pub fn from_json(bytes: &[u8]) -> Result<Self, UpdateError> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|error| UpdateError::Parse(error.to_string()))?;
        if manifest.schema != UPDATE_SCHEMA || manifest.schema_version != UPDATE_SCHEMA_VERSION {
            return Err(UpdateError::UnsupportedSchema {
                schema: manifest.schema,
                version: manifest.schema_version,
            });
        }
        manifest.check().map_err(UpdateError::Parse)?;
        Ok(manifest)
    }

    fn check(&self) -> Result<(), String> {
        Version::parse(&self.application.version).ok_or_else(|| {
            format!(
                "version `{}` is not dotted numeric",
                self.application.version
            )
        })?;
        if let Some(minimum) = &self.minimum_from_version {
            Version::parse(minimum)
                .ok_or_else(|| format!("minimum_from_version `{minimum}` is not dotted numeric"))?;
        }
        if self.channel.is_empty() || self.channel.chars().any(char::is_control) {
            return Err("channel must be a non-empty name".into());
        }
        if crate::from_hex::<32>(&self.package_manifest_blake3).is_none() {
            return Err("package_manifest_blake3 is not a BLAKE3 hex hash".into());
        }
        if self.artifacts.is_empty() {
            return Err("an update has at least one artifact".into());
        }
        for (index, artifact) in self.artifacts.iter().enumerate() {
            if artifact.file.is_empty()
                || artifact.file.contains(['/', '\\', ':'])
                || artifact.file == "."
                || artifact.file == ".."
                || artifact.file.chars().any(char::is_control)
            {
                return Err(format!("artifact `{}` is not a file name", artifact.file));
            }
            if crate::from_hex::<32>(&artifact.blake3).is_none() {
                return Err(format!("artifact `{}`: blake3 is not hex", artifact.file));
            }
            if self.artifacts[..index]
                .iter()
                .any(|other| other.file == artifact.file)
            {
                return Err(format!("artifact `{}` is listed twice", artifact.file));
            }
        }
        Ok(())
    }

    /// Accept an update for the installed package, or say why not. The
    /// signature is checked before a byte is parsed; then identity, target,
    /// channel and version order.
    pub fn verify(
        bytes: &[u8],
        signature: &[u8],
        installed: &PackageManifest,
        policy: &UpdatePolicy,
    ) -> Result<Self, UpdateError> {
        if bytes.len() as u64 > MAX_UPDATE_BYTES {
            return Err(UpdateError::TooLarge);
        }
        let signature: [u8; 64] = signature
            .try_into()
            .map_err(|_| UpdateError::SignatureInvalid)?;
        if !policy
            .publisher
            .verify(&Self::signing_message(bytes), &signature)
        {
            return Err(UpdateError::SignatureInvalid);
        }
        let update = Self::from_json(bytes)?;
        if update.application.id != installed.application.id {
            return Err(UpdateError::WrongApplication {
                expected: installed.application.id.clone(),
                found: update.application.id,
            });
        }
        if update.target.triple != installed.target.triple {
            return Err(UpdateError::WrongTarget {
                expected: installed.target.triple.clone(),
                found: update.target.triple,
            });
        }
        if update.channel != policy.channel {
            return Err(UpdateError::WrongChannel {
                expected: policy.channel.clone(),
                found: update.channel,
            });
        }
        let current = Version::parse(&installed.application.version).ok_or_else(|| {
            UpdateError::Parse(format!(
                "installed version `{}` is not dotted numeric",
                installed.application.version
            ))
        })?;
        let offered = Version::parse(&update.application.version).expect("checked by from_json");
        if offered <= current {
            return Err(UpdateError::NotNewer {
                installed: installed.application.version.clone(),
                offered: update.application.version,
            });
        }
        if let Some(minimum) = &update.minimum_from_version
            && current < Version::parse(minimum).expect("checked by from_json")
        {
            return Err(UpdateError::InstalledTooOld {
                installed: installed.application.version.clone(),
                minimum: minimum.clone(),
            });
        }
        Ok(update)
    }

    pub fn artifact(&self, file: &str) -> Option<&UpdateArtifact> {
        self.artifacts.iter().find(|artifact| artifact.file == file)
    }

    /// Check a downloaded artifact against its size and hash, streaming.
    /// Reads at most one byte past the announced size.
    pub fn check_artifact(&self, file: &str, reader: impl Read) -> Result<(), UpdateError> {
        let artifact = self
            .artifact(file)
            .ok_or_else(|| UpdateError::UnknownArtifact(file.to_owned()))?;
        let mut hasher = blake3::Hasher::new();
        let mut limited = reader.take(artifact.size.saturating_add(1));
        let mut buffer = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let read = limited
                .read(&mut buffer)
                .map_err(|error| UpdateError::Io(error.to_string()))?;
            if read == 0 {
                break;
            }
            total += read as u64;
            hasher.update(&buffer[..read]);
        }
        let expected = crate::from_hex::<32>(&artifact.blake3).expect("checked by from_json");
        if total != artifact.size || *hasher.finalize().as_bytes() != expected {
            return Err(UpdateError::ArtifactMismatch(file.to_owned()));
        }
        Ok(())
    }
}

/// A dotted numeric version with an optional `-prerelease` and ignored
/// `+build` metadata, ordered as SemVer orders it. One to four numeric
/// components; missing ones are zero (`1.2` == `1.2.0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    numbers: [u64; 4],
    pre: Vec<String>,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.split_once('+').map_or(text, |(core, _)| core);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut numbers = [0u64; 4];
        for (index, part) in core.split('.').enumerate() {
            if index == 4
                || part.is_empty()
                || !part.bytes().all(|b| b.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
            {
                return None;
            }
            numbers[index] = part.parse().ok()?;
        }
        let pre = match pre {
            Some(pre) => {
                let parts: Vec<String> = pre.split('.').map(str::to_owned).collect();
                if parts.iter().any(|part| {
                    part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                }) {
                    return None;
                }
                parts
            }
            None => Vec::new(),
        };
        Some(Self { numbers, pre })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.numbers.cmp(&other.numbers).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // A release is newer than any of its prereleases.
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(a), Ok(b)) => a.cmp(&b),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            }
        })
    }
}

/// One installed version, as the journal records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalVersion {
    pub version: String,
    pub build_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JournalState {
    /// Downloaded and verified; files not swapped yet.
    Staged,
    /// Files swapped; the pending version has not confirmed yet.
    Activated,
    /// The pending version reached its healthy point.
    Confirmed,
    /// The previous version was restored.
    RolledBack,
}

/// What a launch should do about an update in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchDecision {
    /// Nothing to do.
    Continue,
    /// Running the pending version: call [`UpdateJournal::confirm`] once the
    /// application is healthy (its first `UiReady`).
    AwaitConfirmation { launch: u32 },
    /// The pending version failed to confirm too often: run the
    /// application's restore hook for `restore`, then
    /// [`UpdateJournal::rolled_back`].
    RollBack { restore: JournalVersion },
    /// The running version is neither the previous nor the pending one; the
    /// journal belongs to another installation and can be discarded.
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollbackPolicy {
    /// Launches of the pending version allowed to end before `UiReady`.
    pub max_unconfirmed_launches: u32,
}

impl Default for RollbackPolicy {
    fn default() -> Self {
        Self {
            max_unconfirmed_launches: 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateJournal {
    pub schema: String,
    pub schema_version: u32,
    pub previous: JournalVersion,
    pub pending: JournalVersion,
    pub state: JournalState,
    pub unconfirmed_launches: u32,
}

impl UpdateJournal {
    /// Written by the updater after the update verified, before any file is
    /// replaced.
    pub fn staged(previous: JournalVersion, pending: JournalVersion) -> Self {
        Self {
            schema: JOURNAL_SCHEMA.into(),
            schema_version: JOURNAL_SCHEMA_VERSION,
            previous,
            pending,
            state: JournalState::Staged,
            unconfirmed_launches: 0,
        }
    }

    /// The files of the pending version are in place.
    pub fn activated(&mut self) {
        if self.state == JournalState::Staged {
            self.state = JournalState::Activated;
            self.unconfirmed_launches = 0;
        }
    }

    /// Call early in every launch, before any window, and persist the
    /// journal afterwards: the count must survive a crash of this launch.
    pub fn begin_launch(&mut self, running: &str, policy: &RollbackPolicy) -> LaunchDecision {
        let is_pending = running == self.pending.version;
        let is_previous = running == self.previous.version;
        if !is_pending && !is_previous {
            return LaunchDecision::Stale;
        }
        match self.state {
            JournalState::Confirmed | JournalState::RolledBack => LaunchDecision::Continue,
            JournalState::Staged if is_previous => LaunchDecision::Continue,
            JournalState::Staged | JournalState::Activated => {
                if is_previous {
                    // The swap was undone outside the journal (an installer
                    // repair, a manual reinstall): nothing is pending now.
                    self.state = JournalState::RolledBack;
                    return LaunchDecision::Continue;
                }
                self.state = JournalState::Activated;
                self.unconfirmed_launches = self.unconfirmed_launches.saturating_add(1);
                if self.unconfirmed_launches > policy.max_unconfirmed_launches {
                    LaunchDecision::RollBack {
                        restore: self.previous.clone(),
                    }
                } else {
                    LaunchDecision::AwaitConfirmation {
                        launch: self.unconfirmed_launches,
                    }
                }
            }
        }
    }

    /// The running pending version reached its healthy point. Returns
    /// whether that changed the journal.
    pub fn confirm(&mut self, running: &str) -> bool {
        if self.state == JournalState::Activated && running == self.pending.version {
            self.state = JournalState::Confirmed;
            self.unconfirmed_launches = 0;
            true
        } else {
            false
        }
    }

    /// The restore hook finished.
    pub fn rolled_back(&mut self) {
        self.state = JournalState::RolledBack;
    }

    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("journal types always serialize");
        text.push('\n');
        text
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, UpdateError> {
        let journal: Self =
            serde_json::from_slice(bytes).map_err(|error| UpdateError::Parse(error.to_string()))?;
        if journal.schema != JOURNAL_SCHEMA || journal.schema_version != JOURNAL_SCHEMA_VERSION {
            return Err(UpdateError::UnsupportedSchema {
                schema: journal.schema,
                version: journal.schema_version,
            });
        }
        Ok(journal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key() -> (SigningKey, PublisherKey) {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let public = PublisherKey::from_bytes(&signing.verifying_key().to_bytes()).unwrap();
        (signing, public)
    }

    fn update(version: &str) -> UpdateManifest {
        UpdateManifest {
            schema: UPDATE_SCHEMA.into(),
            schema_version: UPDATE_SCHEMA_VERSION,
            application: UpdateApplication {
                id: "dev.nana.fixture".into(),
                version: version.into(),
                build_id: None,
            },
            channel: "stable".into(),
            target: ManifestTarget {
                triple: "x86_64-pc-windows-msvc".into(),
                platform: "windows".into(),
            },
            minimum_from_version: None,
            package_manifest_blake3: crate::to_hex(&[1u8; 32]),
            artifacts: vec![UpdateArtifact {
                file: "Fixture-setup.exe".into(),
                format: ArtifactFormat::Nsis,
                size: 5,
                blake3: crate::to_hex(&crate::hash::content(b"hello")),
            }],
        }
    }

    fn signed(update: &UpdateManifest, signing: &SigningKey) -> (Vec<u8>, Vec<u8>) {
        let bytes = update.to_json().into_bytes();
        let signature = signing
            .sign(&UpdateManifest::signing_message(&bytes))
            .to_bytes()
            .to_vec();
        (bytes, signature)
    }

    fn policy(publisher: PublisherKey) -> UpdatePolicy {
        UpdatePolicy {
            channel: "stable".into(),
            publisher,
        }
    }

    #[test]
    fn versions_order_like_semver() {
        let v = |text| Version::parse(text).unwrap();
        assert!(v("1.2.10") > v("1.2.9"));
        assert_eq!(v("1.2"), v("1.2.0"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.2") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.9"));
        assert!(v("1.0.0-beta") > v("1.0.0-alpha.1"));
        assert!(v("1.0.0-alpha.1") > v("1.0.0-alpha"));
        assert_eq!(v("1.0.0+abc"), v("1.0.0+def"));
        for bad in ["", "1..2", "01.2", "1.2.3.4.5", "x", "1.0-", "1.0-a..b"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_signed_newer_update_verifies_and_checks_artifacts() {
        let (signing, public) = key();
        let installed = crate::manifest::tests::sample();
        let (bytes, signature) = signed(&update("1.1.0"), &signing);
        let accepted =
            UpdateManifest::verify(&bytes, &signature, &installed, &policy(public)).unwrap();
        accepted
            .check_artifact("Fixture-setup.exe", &b"hello"[..])
            .unwrap();
        assert_eq!(
            accepted.check_artifact("Fixture-setup.exe", &b"hellO"[..]),
            Err(UpdateError::ArtifactMismatch("Fixture-setup.exe".into()))
        );
        assert_eq!(
            accepted.check_artifact("Fixture-setup.exe", &b"hello!"[..]),
            Err(UpdateError::ArtifactMismatch("Fixture-setup.exe".into()))
        );
        assert!(matches!(
            accepted.check_artifact("other.exe", &b""[..]),
            Err(UpdateError::UnknownArtifact(_))
        ));
    }

    #[test]
    fn signature_identity_channel_and_order_are_enforced() {
        let (signing, public) = key();
        let installed = crate::manifest::tests::sample();
        let policy = policy(public);
        let verify = |update: &UpdateManifest| {
            let (bytes, signature) = signed(update, &signing);
            UpdateManifest::verify(&bytes, &signature, &installed, &policy)
        };

        // Tampered bytes and a signature from another key.
        let (mut bytes, signature) = signed(&update("1.1.0"), &signing);
        bytes[10] ^= 1;
        assert_eq!(
            UpdateManifest::verify(&bytes, &signature, &installed, &policy),
            Err(UpdateError::SignatureInvalid)
        );
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let (bytes, signature) = signed(&update("1.1.0"), &other);
        assert_eq!(
            UpdateManifest::verify(&bytes, &signature, &installed, &policy),
            Err(UpdateError::SignatureInvalid)
        );
        // A package-manifest signature never verifies as an update one.
        let bytes = update("1.1.0").to_json().into_bytes();
        let cross = signing
            .sign(&PackageManifest::signing_message(&bytes))
            .to_bytes();
        assert_eq!(
            UpdateManifest::verify(&bytes, &cross, &installed, &policy),
            Err(UpdateError::SignatureInvalid)
        );

        // Downgrade and replay of the installed release.
        assert!(matches!(
            verify(&update("1.0.0")),
            Err(UpdateError::NotNewer { .. })
        ));
        assert!(matches!(
            verify(&update("0.9.0")),
            Err(UpdateError::NotNewer { .. })
        ));
        let mut wrong = update("1.1.0");
        wrong.application.id = "dev.nana.other".into();
        assert!(matches!(
            verify(&wrong),
            Err(UpdateError::WrongApplication { .. })
        ));
        let mut wrong = update("1.1.0");
        wrong.target.triple = "aarch64-apple-darwin".into();
        assert!(matches!(
            verify(&wrong),
            Err(UpdateError::WrongTarget { .. })
        ));
        let mut wrong = update("1.1.0");
        wrong.channel = "beta".into();
        assert!(matches!(
            verify(&wrong),
            Err(UpdateError::WrongChannel { .. })
        ));
        let mut gated = update("2.0.0");
        gated.minimum_from_version = Some("1.5.0".into());
        assert!(matches!(
            verify(&gated),
            Err(UpdateError::InstalledTooOld { .. })
        ));
        let mut bad = update("1.1.0");
        bad.artifacts[0].file = "../evil.exe".into();
        assert!(matches!(verify(&bad), Err(UpdateError::Parse(_))));
    }

    fn journal() -> UpdateJournal {
        UpdateJournal::staged(
            JournalVersion {
                version: "1.0.0".into(),
                build_id: None,
            },
            JournalVersion {
                version: "1.1.0".into(),
                build_id: None,
            },
        )
    }

    #[test]
    fn a_confirmed_update_stays() {
        let policy = RollbackPolicy::default();
        let mut journal = journal();
        assert_eq!(
            journal.begin_launch("1.0.0", &policy),
            LaunchDecision::Continue
        );
        journal.activated();
        assert_eq!(
            journal.begin_launch("1.1.0", &policy),
            LaunchDecision::AwaitConfirmation { launch: 1 }
        );
        assert!(journal.confirm("1.1.0"));
        assert!(!journal.confirm("1.1.0"));
        for _ in 0..5 {
            assert_eq!(
                journal.begin_launch("1.1.0", &policy),
                LaunchDecision::Continue
            );
        }
        let round_trip = UpdateJournal::from_json(journal.to_json().as_bytes()).unwrap();
        assert_eq!(round_trip, journal);
    }

    #[test]
    fn an_update_that_never_confirms_rolls_back() {
        let policy = RollbackPolicy {
            max_unconfirmed_launches: 2,
        };
        let mut journal = journal();
        journal.activated();
        assert_eq!(
            journal.begin_launch("1.1.0", &policy),
            LaunchDecision::AwaitConfirmation { launch: 1 }
        );
        // The count survives a crash because the caller persisted it.
        let mut journal = UpdateJournal::from_json(journal.to_json().as_bytes()).unwrap();
        assert_eq!(
            journal.begin_launch("1.1.0", &policy),
            LaunchDecision::AwaitConfirmation { launch: 2 }
        );
        assert_eq!(
            journal.begin_launch("1.1.0", &policy),
            LaunchDecision::RollBack {
                restore: JournalVersion {
                    version: "1.0.0".into(),
                    build_id: None
                }
            }
        );
        journal.rolled_back();
        assert_eq!(
            journal.begin_launch("1.0.0", &policy),
            LaunchDecision::Continue
        );
        assert!(!journal.confirm("1.1.0"));
        assert_eq!(
            journal.begin_launch("3.0.0", &policy),
            LaunchDecision::Stale
        );
    }

    #[test]
    fn an_installer_that_skipped_activation_still_counts_launches() {
        let policy = RollbackPolicy::default();
        let mut journal = journal();
        assert_eq!(
            journal.begin_launch("1.1.0", &policy),
            LaunchDecision::AwaitConfirmation { launch: 1 }
        );
        assert_eq!(journal.state, JournalState::Activated);
        // Reverted outside the journal.
        assert_eq!(
            journal.begin_launch("1.0.0", &policy),
            LaunchDecision::Continue
        );
        assert_eq!(journal.state, JournalState::RolledBack);
    }
}
