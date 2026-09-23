//! `nana-package.toml`: what to package and how.
//!
//! Every table denies unknown keys. Credentials never appear here: content
//! keys and the publisher signing key come from the environment or from key
//! files named on the command line (see [`crate::secrets`]); a key whose name
//! looks like a credential is rejected with a pointer to that route.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nana_package::ResourceClass;
use nana_package::manifest::DistributionBackend;
use serde::Deserialize;

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageConfig {
    pub schema_version: u32,
    pub application: ApplicationConfig,
    pub executable: ExecutableConfig,
    pub distribution: DistributionConfig,
    #[serde(default)]
    pub platform: PlatformConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    pub resources: Option<ResourcesConfig>,
    #[serde(default)]
    pub keys: BTreeMap<String, KeyConfig>,
    #[serde(default)]
    pub signing: SigningConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationConfig {
    pub id: String,
    pub name: String,
    pub version: String,
    pub vendor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableConfig {
    /// Product file name without extension (`App` → `App.exe` on Windows).
    pub file_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistributionConfig {
    pub backend: DistributionBackend,
    #[serde(default)]
    pub self_update: bool,
    pub steam: Option<SteamConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteamConfig {
    pub app_id: u32,
    /// Platform (`windows` | `macos` | `linux`) → depot id.
    pub depots: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformConfig {
    pub macos: Option<MacosConfig>,
    pub windows: Option<WindowsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacosConfig {
    pub bundle_identifier: Option<String>,
    #[serde(default = "default_minimum_macos")]
    pub minimum_system_version: String,
    /// `.icns` path; the Nana default mark when absent.
    pub icon: Option<PathBuf>,
    pub category: Option<String>,
    #[serde(default)]
    pub url_schemes: Vec<String>,
    #[serde(default)]
    pub document_types: Vec<DocumentType>,
    /// Plist `<key>…</key><value/>` pairs merged last; a key here replaces
    /// the generated one (the platform-specific override).
    pub info_plist_extra: Option<PathBuf>,
    #[serde(default = "yes")]
    pub strip: bool,
}

impl Default for MacosConfig {
    fn default() -> Self {
        Self {
            bundle_identifier: None,
            minimum_system_version: default_minimum_macos(),
            icon: None,
            category: None,
            url_schemes: Vec::new(),
            document_types: Vec::new(),
            info_plist_extra: None,
            strip: true,
        }
    }
}

fn default_minimum_macos() -> String {
    "11.0".into()
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentType {
    pub name: String,
    pub extensions: Vec<String>,
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "Viewer".into()
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsConfig {
    /// Files allowed next to `App.exe`, each with the constraint that
    /// requires it (e.g. `steam_api64.dll`, which Steamworks loads from the
    /// executable directory).
    #[serde(default)]
    pub root_exceptions: Vec<RootExceptionConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootExceptionConfig {
    pub source: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub bin: Vec<RuntimeFileConfig>,
    #[serde(default)]
    pub tools: Vec<RuntimeFileConfig>,
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFileConfig {
    pub source: PathBuf,
    /// Platforms this file ships on; all when empty.
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    pub name: String,
    pub source: PathBuf,
    pub abi: String,
    pub version: String,
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesConfig {
    /// Directory whose relative paths are the logical resource paths.
    pub root: PathBuf,
    #[serde(default = "default_block_size")]
    pub block_size: u32,
    #[serde(default)]
    pub compression: CompressionConfig,
    /// Free space ratio above which a rebuild compacts (a large delta).
    #[serde(default = "default_max_free_ratio")]
    pub max_free_ratio: f64,
    pub packs: Vec<PackConfig>,
}

fn default_block_size() -> u32 {
    nana_package::pack::format::DEFAULT_BLOCK_SIZE
}

fn default_max_free_ratio() -> f64 {
    0.25
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Codec {
    None,
    Zstd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompressionConfig {
    pub codec: Codec,
    #[serde(default = "default_level")]
    pub level: i32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            codec: Codec::Zstd,
            level: default_level(),
        }
    }
}

fn default_level() -> i32 {
    19
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackConfig {
    pub name: String,
    pub class: ResourceClass,
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Name of a `[keys.*]` entry; unencrypted when absent.
    pub key: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub compression: Option<CompressionConfig>,
    /// Refuse to build a pack larger than this (bytes).
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyAvailability {
    /// Compiled into the application.
    Embedded,
    /// Obtained at process start without UI (local license file, Steam API).
    ProcessStart,
    /// Obtained through a flow the BootstrapUI drives (login, activation).
    AfterBootstrapUi,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyConfig {
    pub generation: u32,
    pub availability: KeyAvailability,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningConfig {
    pub publisher: Option<PublisherConfig>,
    /// Platform signing adapters to run (`macos-codesign`,
    /// `macos-notarize`, `windows-authenticode`, `msix`, `linux-package`).
    #[serde(default)]
    pub platform: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherConfig {
    /// `ed25519:<hex>`; the signing key supplied at package time must match.
    /// Without it any supplied key signs (and the build says it is unpinned).
    pub public_key: Option<String>,
    #[serde(default = "yes")]
    pub required: bool,
}

/// Field names that suggest a credential was put in the config.
const SECRET_WORDS: [&str; 5] = ["secret", "private", "token", "password", "credential"];

impl PackageConfig {
    pub fn load(path: &Path) -> Result<(Self, PathBuf), String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let base = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Ok((Self::parse(&text)?, base))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let value: toml::Table =
            toml::from_str(text).map_err(|error| format!("nana-package.toml: {error}"))?;
        reject_secret_fields(&value, "")?;
        let config: Self = value
            .try_into()
            .map_err(|error| format!("nana-package.toml: {error}"))?;
        config.check()?;
        Ok(config)
    }

    fn check(&self) -> Result<(), String> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(format!(
                "nana-package.toml schema_version {} is not supported (expected {CONFIG_SCHEMA_VERSION})",
                self.schema_version
            ));
        }
        if self.distribution.self_update {
            return Err(
                "distribution.self_update: the self-update backend is not implemented yet \
                 (Issue #226 workstream G); Steam and installer builds do not carry one"
                    .into(),
            );
        }
        if self.distribution.backend == DistributionBackend::Steam
            && self.distribution.steam.is_none()
        {
            return Err("distribution.backend = \"steam\" needs [distribution.steam]".into());
        }
        let file_name = &self.executable.file_name;
        if file_name.is_empty()
            || file_name.contains(['/', '\\', '.', ':'])
            || file_name.trim() != file_name
        {
            return Err(format!(
                "executable.file_name `{file_name}` must be a bare name without extension"
            ));
        }
        let name = &self.application.name;
        if name.trim().is_empty()
            || name.trim() != name
            || name.starts_with('.')
            || name.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
            || name.chars().any(char::is_control)
        {
            return Err(format!(
                "application.name `{name}` cannot name a bundle or folder (no path separators, \
                 reserved characters or leading dot)"
            ));
        }
        const PLATFORMS: [&str; 3] = ["windows", "macos", "linux"];
        let platform_lists = self
            .runtime
            .bin
            .iter()
            .chain(&self.runtime.tools)
            .map(|file| &file.platforms)
            .chain(self.runtime.plugins.iter().map(|plugin| &plugin.platforms));
        for list in platform_lists {
            if let Some(unknown) = list.iter().find(|p| !PLATFORMS.contains(&p.as_str())) {
                return Err(format!(
                    "unknown platform `{unknown}` (windows | macos | linux)"
                ));
            }
        }
        if let Some(steam) = &self.distribution.steam
            && let Some(unknown) = steam
                .depots
                .keys()
                .find(|p| !PLATFORMS.contains(&p.as_str()))
        {
            return Err(format!(
                "[distribution.steam].depots: unknown platform `{unknown}` (windows | macos | linux)"
            ));
        }
        for key in self.keys.values() {
            if key.generation == 0 {
                return Err("keys.*.generation starts at 1".into());
            }
        }
        if let Some(resources) = &self.resources {
            let block = resources.block_size;
            if !block.is_power_of_two()
                || !(nana_package::pack::format::MIN_BLOCK_SIZE
                    ..=nana_package::pack::format::MAX_BLOCK_SIZE)
                    .contains(&block)
            {
                return Err(format!(
                    "resources.block_size {block} must be a power of two between 4 KiB and 4 MiB"
                ));
            }
            if !(0.0..=1.0).contains(&resources.max_free_ratio) {
                return Err("resources.max_free_ratio must be within 0..=1".into());
            }
        }
        Ok(())
    }
}

fn reject_secret_fields(table: &toml::Table, path: &str) -> Result<(), String> {
    for (key, value) in table {
        let full = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        // Names of keys and depots are identifiers, not fields.
        let is_name = path == "keys" || path == "distribution.steam.depots";
        let lower = key.to_ascii_lowercase();
        if !is_name && SECRET_WORDS.iter().any(|word| lower.contains(word)) {
            return Err(format!(
                "nana-package.toml: `{full}` looks like a credential. Content keys and signing \
                 keys are never stored in the package config: pass them through \
                 NANA_CONTENT_KEY_<NAME> / NANA_PUBLISHER_SIGNING_KEY or \
                 --content-key-file / --signing-key-file"
            ));
        }
        match value {
            toml::Value::Table(inner) => reject_secret_fields(inner, &full)?,
            toml::Value::Array(items) => {
                for item in items {
                    if let toml::Value::Table(inner) = item {
                        reject_secret_fields(inner, &full)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const MINIMAL: &str = r#"
schema_version = 1

[application]
id = "dev.nana.fixture"
name = "Nana Fixture"
version = "0.1.0"

[executable]
file_name = "NanaFixture"

[distribution]
backend = "steam"

[distribution.steam]
app_id = 480
depots = { windows = 481, macos = 482, linux = 483 }
"#;

    #[test]
    fn minimal_config_parses() {
        let config = PackageConfig::parse(MINIMAL).unwrap();
        assert_eq!(config.application.id, "dev.nana.fixture");
        assert_eq!(config.distribution.backend, DistributionBackend::Steam);
        assert!(config.resources.is_none());
    }

    #[test]
    fn credentials_are_rejected_with_guidance() {
        let text = format!(
            "{MINIMAL}\n[signing.publisher]\npublic_key = \"ed25519:00\"\nprivate_key = \"abc\"\n"
        );
        let error = PackageConfig::parse(&text).unwrap_err();
        assert!(error.contains("signing.publisher.private_key"), "{error}");
        assert!(error.contains("NANA_PUBLISHER_SIGNING_KEY"), "{error}");
        let text = MINIMAL.replace("app_id = 480", "app_id = 480\nupload_token = \"x\"");
        assert!(
            PackageConfig::parse(&text)
                .unwrap_err()
                .contains("credential")
        );
    }

    #[test]
    fn unknown_fields_and_self_update_are_rejected() {
        let text = MINIMAL.replace(
            "file_name = \"NanaFixture\"",
            "file_name = \"NanaFixture\"\nicon = \"x\"",
        );
        assert!(PackageConfig::parse(&text).is_err());
        let text = MINIMAL.replace(
            "backend = \"steam\"",
            "backend = \"steam\"\nself_update = true",
        );
        assert!(
            PackageConfig::parse(&text)
                .unwrap_err()
                .contains("self-update")
        );
        let text = MINIMAL.replace("file_name = \"NanaFixture\"", "file_name = \"App.exe\"");
        assert!(PackageConfig::parse(&text).is_err());
    }

    #[test]
    fn names_and_platforms_are_checked() {
        let text = MINIMAL.replace("name = \"Nana Fixture\"", "name = \"../evil\"");
        assert!(
            PackageConfig::parse(&text)
                .unwrap_err()
                .contains("application.name")
        );
        let text = MINIMAL.replace("windows = 481", "window = 481");
        assert!(PackageConfig::parse(&text).unwrap_err().contains("window"));
        let text = format!(
            "{MINIMAL}\n[runtime]\ntools = [{{ source = \"t\", platforms = [\"win\"] }}]\n"
        );
        assert!(PackageConfig::parse(&text).unwrap_err().contains("win"));
    }

    #[test]
    fn steam_backend_requires_steam_table() {
        let text = MINIMAL
            .split("[distribution.steam]")
            .next()
            .unwrap()
            .to_owned();
        assert!(
            PackageConfig::parse(&text)
                .unwrap_err()
                .contains("[distribution.steam]")
        );
    }
}
