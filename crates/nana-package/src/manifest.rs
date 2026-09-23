//! The package manifest: `<RuntimeManifest>/package.json`, optionally with a
//! detached Ed25519 signature in `package.json.sig`.
//!
//! It records what was built and how it was packaged, for crash reports,
//! diagnostics, compatibility checks and final-artifact validation. Every
//! struct denies unknown fields and none has a free-form map, so there is no
//! place for a key, token or credential to hide; the packager additionally
//! scans every emitted file for the secrets it loaded.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::keys::TrustPolicy;
use crate::pack::ResourceClass;

pub const MANIFEST_FILE: &str = "package.json";
pub const MANIFEST_SIGNATURE_FILE: &str = "package.json.sig";
pub const MANIFEST_SCHEMA: &str = "nana.package-manifest";
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_SIGNATURE_CONTEXT: &[u8] = b"nana.package-manifest.v1\0";
/// Refuse to parse anything larger.
pub const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    pub schema: String,
    pub schema_version: u32,
    pub application: ManifestApplication,
    pub target: ManifestTarget,
    pub nana: ManifestNana,
    pub build: ManifestBuild,
    pub static_app_plan: StaticAppPlanStatus,
    pub distribution: ManifestDistribution,
    pub layout: ManifestLayout,
    pub resource_packs: Vec<ManifestPack>,
    pub plugins: Vec<ManifestPlugin>,
    pub runtime_files: Vec<ManifestFile>,
    pub signing: SigningReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestApplication {
    pub id: String,
    pub name: String,
    pub version: String,
    pub vendor: Option<String>,
    pub build_id: Option<String>,
}

impl ManifestApplication {
    /// The one identity rule packager, validator and runtime share: id,
    /// name and version equal; an empty vendor and no vendor are the same.
    pub fn matches(&self, id: &str, name: &str, version: &str, vendor: Option<&str>) -> bool {
        self.id == id
            && self.name == name
            && self.version == version
            && self.vendor.as_deref().unwrap_or_default() == vendor.unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestTarget {
    /// Rust target triple, e.g. `x86_64-pc-windows-msvc`.
    pub triple: String,
    /// `windows` | `macos` | `linux`.
    pub platform: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestNana {
    /// Version of the packaging contract (`nana-package`).
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestBuild {
    /// Cargo profile the executable was built with, as declared to the
    /// packager (`dist` for distribution builds).
    pub profile: String,
    /// The executable carries overflow-check panic text: always true of a
    /// dev-profile build, occasionally of a release one. A hint, not proof.
    pub debug_assertions_suspected: bool,
}

/// The StaticAppPlan fingerprint (Issue #158). Recorded as unavailable until
/// the dist pipeline produces one; never invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StaticAppPlanStatus {
    Unavailable { reason: String },
    Present { fingerprint: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DistributionBackend {
    Steam,
    Installer,
    Portable,
}

impl DistributionBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::Installer => "installer",
            Self::Portable => "portable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestDistribution {
    pub backend: DistributionBackend,
    /// Whether the package carries a self-update helper. Always false for
    /// Steam; see Issue #226 §8.
    pub self_update: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestLayout {
    /// Executable path relative to the application root, `/`-separated.
    pub executable: String,
    /// Logical location → path relative to the application root.
    pub runtime_bin: String,
    pub runtime_resources: String,
    pub runtime_plugins: String,
    pub runtime_tools: String,
    pub runtime_manifest: String,
    /// Files allowed in the application root besides the executable, each
    /// with the platform or third-party constraint that requires it.
    pub root_exceptions: Vec<RootException>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootException {
    pub file: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestPack {
    pub name: String,
    /// File name inside RuntimeResources.
    pub file: String,
    pub class: ResourceClass,
    /// Logical path prefixes routed to this pack (longest match wins).
    pub prefixes: Vec<String>,
    pub format_version: u16,
    /// Hex.
    pub pack_id: String,
    /// Hex BLAKE3 of the stored TOC; pins the exact pack build.
    pub toc_hash: String,
    pub size: u64,
    pub entries: u32,
    /// Name of the content key in `nana-package.toml` (an identifier, never
    /// the key), so tools can ask for it; `None` when not encrypted.
    pub key_name: Option<String>,
    /// Hex content key id; `None` when not encrypted.
    pub key_id: Option<String>,
    pub key_generation: u32,
    pub signed: bool,
    /// Packs this one may reference; each of the same or an earlier class.
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestPlugin {
    pub name: String,
    /// Path relative to RuntimePlugins.
    pub file: String,
    pub abi: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileKind {
    Executable,
    Library,
    Tool,
    Plugin,
    Resource,
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    /// Path relative to the application root, `/`-separated.
    pub path: String,
    pub kind: FileKind,
    pub size: u64,
    /// Hex BLAKE3. For executables and libraries this is the hash before
    /// platform code signing, which rewrites the file afterwards.
    pub blake3: String,
    pub pre_platform_signing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningReport {
    /// Resource-publisher signature over packs and this manifest.
    pub publisher: SignStatus,
    /// `ed25519:<hex>` public key, when signed.
    pub publisher_key: Option<String>,
    /// Platform signing adapters (codesign, Authenticode, ...).
    pub platform: Vec<PlatformSigning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformSigning {
    pub adapter: String,
    pub status: SignStatus,
}

/// What happened to one signature. "Not executed" is reported as such, never
/// as success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SignStatus {
    NotConfigured,
    NotExecuted { reason: String },
    Signed,
    Verified,
    Failed { detail: String },
}

impl SignStatus {
    pub fn is_signed(&self) -> bool {
        matches!(self, Self::Signed | Self::Verified)
    }
}

/// How the manifest's signature was treated when it was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureState {
    /// Checked against the pinned publisher key.
    Verified,
    /// No signature file, and the policy allows that.
    Unsigned,
    /// A signature exists but the policy pins no key to check it with.
    NotChecked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    Missing,
    Io(String),
    TooLarge,
    SignatureRequired,
    SignatureInvalid,
    Parse(String),
    UnsupportedSchema { schema: String, version: u32 },
}

impl ManifestError {
    pub const fn code(&self) -> u64 {
        match self {
            Self::Missing => 1,
            Self::Io(_) => 2,
            Self::TooLarge => 3,
            Self::SignatureRequired => 4,
            Self::SignatureInvalid => 5,
            Self::Parse(_) => 6,
            Self::UnsupportedSchema { .. } => 7,
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("package manifest is missing"),
            Self::Io(error) => write!(f, "cannot read package manifest: {error}"),
            Self::TooLarge => f.write_str("package manifest is too large"),
            Self::SignatureRequired => f.write_str("package manifest is not signed"),
            Self::SignatureInvalid => f.write_str("package manifest signature is invalid"),
            Self::Parse(error) => write!(f, "invalid package manifest: {error}"),
            Self::UnsupportedSchema { schema, version } => {
                write!(f, "unsupported package manifest schema {schema} v{version}")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

impl PackageManifest {
    /// Serialized form. Deterministic: field order is declaration order and
    /// every collection is a `Vec` the packager sorts.
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("manifest types always serialize");
        text.push('\n');
        text
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| ManifestError::Parse(error.to_string()))?;
        if manifest.schema != MANIFEST_SCHEMA || manifest.schema_version != MANIFEST_SCHEMA_VERSION
        {
            return Err(ManifestError::UnsupportedSchema {
                schema: manifest.schema,
                version: manifest.schema_version,
            });
        }
        manifest.check().map_err(ManifestError::Parse)?;
        Ok(manifest)
    }

    /// Structural checks serde cannot express. A manifest that fails them is
    /// invalid as a whole, never partially trusted: a pack whose pin cannot be
    /// parsed would otherwise mount unpinned, and a path with `..` would
    /// point outside the package.
    fn check(&self) -> Result<(), String> {
        let layout = &self.layout;
        for path in [
            &layout.executable,
            &layout.runtime_bin,
            &layout.runtime_resources,
            &layout.runtime_plugins,
            &layout.runtime_tools,
            &layout.runtime_manifest,
        ] {
            relative_path(path)?;
        }
        for exception in &layout.root_exceptions {
            file_name(&exception.file)?;
        }
        for pack in &self.resource_packs {
            file_name(&pack.file)?;
            if pack.expected().is_none() {
                return Err(format!(
                    "pack `{}`: pack_id / toc_hash are not hex",
                    pack.name
                ));
            }
            if pack
                .key_id
                .as_deref()
                .is_some_and(|id| crate::from_hex::<8>(id).is_none())
            {
                return Err(format!("pack `{}`: key_id is not hex", pack.name));
            }
            if pack.prefixes.is_empty() {
                return Err(format!("pack `{}` has no prefixes", pack.name));
            }
            for prefix in &pack.prefixes {
                relative_path(prefix.trim_end_matches('/'))?;
            }
        }
        for plugin in &self.plugins {
            relative_path(&plugin.file)?;
        }
        for file in &self.runtime_files {
            relative_path(&file.path)?;
        }
        Ok(())
    }

    /// The message a publisher signs for `manifest_bytes`.
    pub fn signing_message(manifest_bytes: &[u8]) -> Vec<u8> {
        let mut message = MANIFEST_SIGNATURE_CONTEXT.to_vec();
        message.extend_from_slice(manifest_bytes);
        message
    }

    /// Read `<dir>/package.json`, checking its signature as `trust` demands
    /// before parsing a single byte of it.
    pub fn read(dir: &Path, trust: &TrustPolicy) -> Result<(Self, SignatureState), ManifestError> {
        let bytes = read_capped(&dir.join(MANIFEST_FILE), MAX_MANIFEST_BYTES)?
            .ok_or(ManifestError::Missing)?;
        let signature = match read_capped(&dir.join(MANIFEST_SIGNATURE_FILE), 64) {
            Err(ManifestError::TooLarge) => return Err(ManifestError::SignatureInvalid),
            other => other?,
        };
        // A signature file is exactly one Ed25519 signature, whatever the
        // policy, so every reader gives the same answer about it.
        let signature: Option<[u8; 64]> = match signature {
            Some(bytes) => Some(
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| ManifestError::SignatureInvalid)?,
            ),
            None => None,
        };
        let state = match (trust.publisher(), signature) {
            (Some(publisher), Some(signature)) => {
                if !publisher.verify(&Self::signing_message(&bytes), &signature) {
                    return Err(ManifestError::SignatureInvalid);
                }
                SignatureState::Verified
            }
            (Some(_), None) => return Err(ManifestError::SignatureRequired),
            (None, Some(_)) => SignatureState::NotChecked,
            (None, None) => SignatureState::Unsigned,
        };
        Ok((Self::from_json(&bytes)?, state))
    }

    pub fn pack(&self, name: &str) -> Option<&ManifestPack> {
        self.resource_packs.iter().find(|pack| pack.name == name)
    }

    /// The pack a logical path routes to: the longest prefix that
    /// [claims](prefix_claims) it. The runtime's `nana://res/` mount and
    /// the Early Splash read route the same way.
    pub fn route(&self, path: &str) -> Option<&ManifestPack> {
        self.resource_packs
            .iter()
            .flat_map(|pack| pack.prefixes.iter().map(move |prefix| (prefix, pack)))
            .filter(|(prefix, _)| prefix_claims(prefix, path))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, pack)| pack)
    }
}

/// Whether a pack prefix claims `path`: one ending in `/` claims everything
/// under it, any other exactly one path.
pub fn prefix_claims(prefix: &str, path: &str) -> bool {
    if prefix.ends_with('/') {
        path.starts_with(prefix)
    } else {
        path == prefix
    }
}

impl ManifestPack {
    /// Encrypted packs name their content key; plain ones do not.
    pub fn encrypted(&self) -> bool {
        self.key_id.is_some()
    }

    pub fn expected(&self) -> Option<crate::pack::ExpectedPack> {
        Some(crate::pack::ExpectedPack {
            pack_id: crate::from_hex::<16>(&self.pack_id)?,
            toc_hash: crate::from_hex::<32>(&self.toc_hash)?,
            class: self.class,
            encrypted: self.encrypted(),
            signed: self.signed,
            key_generation: self.key_generation,
        })
    }
}

fn relative_path(path: &str) -> Result<(), String> {
    crate::valid_logical_path(path)
        .then_some(())
        .ok_or_else(|| format!("`{path}` is not a relative package path"))
}

fn file_name(name: &str) -> Result<(), String> {
    relative_path(name)?;
    (!name.contains('/'))
        .then_some(())
        .ok_or_else(|| format!("`{name}` must be a file name, not a path"))
}

fn read_capped(path: &Path, cap: u64) -> Result<Option<Vec<u8>>, ManifestError> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ManifestError::Io(error.to_string())),
    };
    let mut bytes = Vec::new();
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| ManifestError::Io(error.to_string()))?;
    if bytes.len() as u64 > cap {
        return Err(ManifestError::TooLarge);
    }
    Ok(Some(bytes))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::keys::PublisherKey;
    use ed25519_dalek::{Signer, SigningKey};

    pub(crate) fn sample() -> PackageManifest {
        PackageManifest {
            schema: MANIFEST_SCHEMA.into(),
            schema_version: MANIFEST_SCHEMA_VERSION,
            application: ManifestApplication {
                id: "dev.nana.fixture".into(),
                name: "Fixture".into(),
                version: "1.0.0".into(),
                vendor: None,
                build_id: Some("42".into()),
            },
            target: ManifestTarget {
                triple: "x86_64-pc-windows-msvc".into(),
                platform: "windows".into(),
            },
            nana: ManifestNana {
                version: crate::NANA_VERSION.into(),
            },
            build: ManifestBuild {
                profile: "dist".into(),
                debug_assertions_suspected: false,
            },
            static_app_plan: StaticAppPlanStatus::Unavailable {
                reason: "Issue #158 not implemented".into(),
            },
            distribution: ManifestDistribution {
                backend: DistributionBackend::Steam,
                self_update: false,
            },
            layout: ManifestLayout {
                executable: "Fixture.exe".into(),
                runtime_bin: "runtime/bin".into(),
                runtime_resources: "runtime/resources".into(),
                runtime_plugins: "runtime/plugins".into(),
                runtime_tools: "runtime/tools".into(),
                runtime_manifest: "runtime/manifest".into(),
                root_exceptions: vec![],
            },
            resource_packs: vec![],
            plugins: vec![],
            runtime_files: vec![],
            signing: SigningReport {
                publisher: SignStatus::NotConfigured,
                publisher_key: None,
                platform: vec![PlatformSigning {
                    adapter: "windows-authenticode".into(),
                    status: SignStatus::NotExecuted {
                        reason: "adapter not implemented".into(),
                    },
                }],
            },
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nana-package-manifest-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn manifest_round_trips_and_rejects_unknown_fields() {
        let manifest = sample();
        let json = manifest.to_json();
        assert_eq!(
            PackageManifest::from_json(json.as_bytes()).unwrap(),
            manifest
        );
        let with_secret = json.replacen("\"schema\"", "\"api_token\": \"x\",\n  \"schema\"", 1);
        assert!(matches!(
            PackageManifest::from_json(with_secret.as_bytes()),
            Err(ManifestError::Parse(_))
        ));
        let other_schema = json.replace("\"schema_version\": 1", "\"schema_version\": 9");
        assert!(matches!(
            PackageManifest::from_json(other_schema.as_bytes()),
            Err(ManifestError::UnsupportedSchema { .. })
        ));
    }

    #[test]
    fn unsafe_paths_and_unparseable_pins_invalidate_the_manifest() {
        let mut manifest = sample();
        manifest.runtime_files.push(ManifestFile {
            path: "../outside".into(),
            kind: FileKind::Resource,
            size: 0,
            blake3: String::new(),
            pre_platform_signing: false,
        });
        assert!(matches!(
            PackageManifest::from_json(manifest.to_json().as_bytes()),
            Err(ManifestError::Parse(_))
        ));
        let mut manifest = sample();
        manifest.resource_packs.push(ManifestPack {
            name: "ui".into(),
            file: "../ui.nrpack".into(),
            class: ResourceClass::Protected,
            prefixes: vec!["ui/".into()],
            format_version: 1,
            pack_id: "00".repeat(16),
            toc_hash: "00".repeat(32),
            size: 0,
            entries: 0,
            key_name: None,
            key_id: None,
            key_generation: 0,
            signed: false,
            depends_on: vec![],
        });
        assert!(PackageManifest::from_json(manifest.to_json().as_bytes()).is_err());
        manifest.resource_packs[0].file = "ui.nrpack".into();
        assert!(PackageManifest::from_json(manifest.to_json().as_bytes()).is_ok());
        manifest.resource_packs[0].toc_hash = "zz".into();
        assert!(PackageManifest::from_json(manifest.to_json().as_bytes()).is_err());
    }

    #[test]
    fn signature_is_checked_before_parsing() {
        let dir = temp_dir("sig");
        let signing = SigningKey::from_bytes(&[5; 32]);
        let publisher = PublisherKey::from_bytes(signing.verifying_key().as_bytes()).unwrap();
        let trust = TrustPolicy::RequirePublisher(publisher);
        let bytes = sample().to_json().into_bytes();
        std::fs::write(dir.join(MANIFEST_FILE), &bytes).unwrap();

        assert_eq!(
            PackageManifest::read(&dir, &trust).unwrap_err(),
            ManifestError::SignatureRequired
        );
        assert_eq!(
            PackageManifest::read(&dir, &TrustPolicy::AllowUnsigned)
                .unwrap()
                .1,
            SignatureState::Unsigned
        );

        let signature = signing.sign(&PackageManifest::signing_message(&bytes));
        std::fs::write(dir.join(MANIFEST_SIGNATURE_FILE), signature.to_bytes()).unwrap();
        assert_eq!(
            PackageManifest::read(&dir, &trust).unwrap().1,
            SignatureState::Verified
        );

        let mut tampered = bytes.clone();
        let at = tampered.len() / 2;
        tampered[at] ^= 1;
        std::fs::write(dir.join(MANIFEST_FILE), &tampered).unwrap();
        assert_eq!(
            PackageManifest::read(&dir, &trust).unwrap_err(),
            ManifestError::SignatureInvalid
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_manifest_is_explicit() {
        let dir = temp_dir("missing");
        assert_eq!(
            PackageManifest::read(&dir, &TrustPolicy::AllowUnsigned).unwrap_err(),
            ManifestError::Missing
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
