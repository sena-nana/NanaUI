//! Installer backends (Issue #274 §2–3): the packaged application tree in
//! the form each platform installs.
//!
//! | format | target | built by | runs on |
//! | --- | --- | --- | --- |
//! | `dmg` | macOS | `hdiutil` (the `.app` plus an `Applications` link) | macOS |
//! | `msi` | Windows | WiX v4+ `wix build` from a generated `.wxs` | Windows |
//! | `nsis` | Windows | `makensis` from a generated `.nsi` | any host with makensis |
//! | `deb` | Linux | the packager itself (FHS layout, pure Rust) | any host |
//! | `appimage` | Linux | `appimagetool` from a generated AppDir | Linux |
//! | `flatpak` | Linux | `flatpak-builder` + `flatpak build-bundle` from a generated manifest | Linux |
//!
//! A format whose tool cannot run here is reported `not-executed` with the
//! reason, and its generated sources (`.wxs`, `.nsi`, AppDir, Flatpak
//! manifest) are kept under `installer/staging/` so the build can finish on
//! the right host. Nothing is reported built that was not built; a tool
//! that runs and fails fails the packaging run.
//!
//! The Linux formats install the tree in FHS form: the executable and its
//! architecture-dependent `runtime/{bin,plugins,tools}` under
//! `<prefix>/lib/<pkg>/`, the architecture-independent `resources` and
//! `manifest` under `<prefix>/share/<pkg>/`, a `<prefix>/bin/<exe>` link,
//! and a desktop entry with its icon. `ApplicationPaths` resolves exactly
//! that layout at run time (`/usr`, AppImage `$APPDIR/usr`, Flatpak `/app`).

pub mod archive;
mod deb;
mod dmg;
mod linux_bundles;
mod nsis;
pub(crate) mod wix;

use std::path::{Path, PathBuf};

use nana_package::update::ArtifactFormat;
use serde::Serialize;

use crate::config::{InstallerConfig, PackageConfig};
use crate::layout::{PackageLayout, TargetPlatform};
use crate::sign::tool::{Environment, Host, ToolCommand, ToolOutput, ToolRunner};
use crate::util::{content_hex, io};

/// The formats that install on `platform`.
pub fn formats_for(platform: TargetPlatform) -> &'static [ArtifactFormat] {
    match platform {
        TargetPlatform::Macos => &[ArtifactFormat::Dmg],
        TargetPlatform::Windows => &[ArtifactFormat::Msi, ArtifactFormat::Nsis],
        TargetPlatform::Linux => &[
            ArtifactFormat::Deb,
            ArtifactFormat::Appimage,
            ArtifactFormat::Flatpak,
        ],
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallerReport {
    pub format: &'static str,
    #[serde(flatten)]
    pub status: InstallerStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum InstallerStatus {
    Built {
        /// Relative to the output directory.
        file: String,
        size: u64,
        blake3: String,
        checks: Vec<ToolCheck>,
    },
    NotExecuted {
        reason: String,
        /// Generated sources kept for a build on the right host.
        staged: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCheck {
    pub name: String,
    #[serde(flatten)]
    pub status: CheckStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", content = "detail", rename_all = "kebab-case")]
pub enum CheckStatus {
    Pass,
    NotExecuted(String),
}

pub struct InstallerInput<'a> {
    pub config: &'a PackageConfig,
    pub installer: &'a InstallerConfig,
    /// Directory relative config paths resolve against.
    pub base: &'a Path,
    pub platform: TargetPlatform,
    pub target: &'a str,
    pub layout: &'a PackageLayout,
    pub app_root: &'a Path,
    /// `<out>`: installers go to `<out>/installer/`.
    pub out: &'a Path,
    pub env: &'a dyn Environment,
    pub runner: &'a dyn ToolRunner,
    pub host: Host,
}

/// What one backend produced.
pub(crate) enum Outcome {
    Built {
        path: PathBuf,
        checks: Vec<ToolCheck>,
    },
    NotExecuted {
        reason: String,
        staged: Option<PathBuf>,
    },
}

/// Metadata every backend shares.
pub(crate) struct Meta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub vendor: String,
    pub summary: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub maintainer: String,
    /// Linux package / directory name.
    pub package: String,
    /// The executable's file name inside the application tree.
    pub exe: String,
    /// A file-name-safe form of the application name.
    pub stem: String,
    pub categories: Vec<String>,
}

impl Meta {
    fn new(input: &InstallerInput<'_>) -> Self {
        let app = &input.config.application;
        let vendor = app
            .vendor
            .clone()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| app.name.clone());
        let installer = input.installer;
        Self {
            id: app.id.clone(),
            name: app.name.clone(),
            version: app.version.clone(),
            summary: installer
                .summary
                .clone()
                .unwrap_or_else(|| app.name.clone()),
            description: installer.description.clone(),
            homepage: installer.homepage.clone(),
            maintainer: installer
                .maintainer
                .clone()
                .unwrap_or_else(|| vendor.clone()),
            vendor,
            package: installer.linux_package.clone().unwrap_or_else(|| {
                derive_linux_package(&app.name, &input.config.executable.file_name)
            }),
            exe: input
                .layout
                .executable
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned(),
            stem: app
                .name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect(),
            categories: if installer.categories.is_empty() {
                vec!["Utility".into()]
            } else {
                installer.categories.clone()
            },
        }
    }
}

/// Debian package-name rules, which Linux directory names here follow.
pub fn valid_linux_package(name: &str) -> bool {
    name.len() >= 2
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+-.".contains(&b))
}

fn derive_linux_package(name: &str, file_name: &str) -> String {
    let derive = |text: &str| {
        let mut out = String::new();
        for c in text.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c.to_ascii_lowercase());
            } else if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
        }
        out.trim_end_matches('-').to_owned()
    };
    let from_name = derive(name);
    if valid_linux_package(&from_name) {
        from_name
    } else {
        let from_file = derive(file_name);
        if valid_linux_package(&from_file) {
            from_file
        } else {
            "nana-app".into()
        }
    }
}

/// `SOURCE_DATE_EPOCH`, for reproducible archive timestamps.
pub(crate) fn source_date(env: &dyn Environment) -> u64 {
    env.var("SOURCE_DATE_EPOCH")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Run a tool that must succeed.
pub(crate) fn run(runner: &dyn ToolRunner, command: &ToolCommand) -> Result<ToolOutput, String> {
    match runner.run(command) {
        Ok(output) if output.success => Ok(output),
        other => Err(crate::sign::tool::failure(command, &other)),
    }
}

/// `program`: `override_var` when set, else on `PATH`.
pub(crate) fn find_tool(
    input: &InstallerInput<'_>,
    program: &str,
    override_var: Option<&str>,
) -> Option<PathBuf> {
    override_var
        .and_then(|var| input.env.var(var))
        .map(PathBuf::from)
        .or_else(|| input.runner.find(program))
}

fn escape_desktop(value: &str) -> String {
    value.replace('\\', "\\\\")
}

/// The freedesktop entry Linux formats install as `<id>.desktop`.
pub(crate) fn desktop_entry(meta: &Meta) -> String {
    let exec = if meta.exe.contains([' ', '"', '\'', '\\', '$', '`']) {
        format!(
            "\"{}\"",
            meta.exe
                .replace('\\', "\\\\\\\\")
                .replace('"', "\\\\\"")
                .replace('$', "\\\\$")
                .replace('`', "\\\\`")
        )
    } else {
        meta.exe.clone()
    };
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.5\nName={}\nComment={}\nExec={exec}\n\
         Icon={}\nTerminal=false\nCategories={};\n",
        escape_desktop(&meta.name),
        escape_desktop(&meta.summary),
        meta.id,
        meta.categories.join(";"),
    )
}

/// Lay the application tree out under `prefix` in FHS form. Returns the
/// symlinks of the layout (`bin/<exe>`), relative to `prefix`; they are
/// created on Unix hosts only (an archive writer adds them elsewhere).
pub(crate) fn stage_fhs(
    input: &InstallerInput<'_>,
    meta: &Meta,
    prefix: &Path,
    icon: &[u8],
    with_bin_link: bool,
) -> Result<Vec<(String, String)>, String> {
    let lib = prefix.join("lib").join(&meta.package);
    let share = prefix.join("share").join(&meta.package);
    crate::util::copy_tree(input.app_root, &lib)
        .map_err(|error| format!("cannot stage the application tree: {error}"))?;
    std::fs::create_dir_all(&share).map_err(io("create share directory"))?;
    for (location, name) in [
        (&input.layout.runtime_manifest, "manifest"),
        (&input.layout.runtime_resources, "resources"),
    ] {
        let from = PackageLayout::join(&lib, location);
        if from.is_dir() {
            std::fs::rename(&from, share.join(name)).map_err(io("move into share"))?;
        } else {
            std::fs::create_dir_all(share.join(name)).map_err(io("create share directory"))?;
        }
    }
    let applications = prefix.join("share/applications");
    std::fs::create_dir_all(&applications).map_err(io("create applications directory"))?;
    std::fs::write(
        applications.join(format!("{}.desktop", meta.id)),
        desktop_entry(meta),
    )
    .map_err(io("write desktop entry"))?;
    let icons = prefix.join("share/icons/hicolor/256x256/apps");
    std::fs::create_dir_all(&icons).map_err(io("create icon directory"))?;
    std::fs::write(icons.join(format!("{}.png", meta.id)), icon).map_err(io("write icon"))?;

    let mut links = Vec::new();
    if with_bin_link {
        let link = format!("bin/{}", meta.exe);
        let target = format!("../lib/{}/{}", meta.package, meta.exe);
        std::fs::create_dir_all(prefix.join("bin")).map_err(io("create bin directory"))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, prefix.join(&link)).map_err(io("link bin"))?;
        links.push((link, target));
    }
    Ok(links)
}

/// The desktop icon: `icon_png`, else the Nana mark.
pub(crate) fn linux_icon(input: &InstallerInput<'_>) -> Result<Vec<u8>, String> {
    match &input.installer.icon_png {
        Some(path) => {
            let bytes = std::fs::read(input.base.join(path))
                .map_err(|error| format!("cannot read icon {}: {error}", path.display()))?;
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err(format!("{} is not a PNG", path.display()));
            }
            Ok(bytes)
        }
        None => nana_app_icon::encode_default_png(256),
    }
}

/// Build every configured installer that belongs to the target. Returns
/// the report and the artifacts built (for the artifact signing stage).
pub fn build_installers(
    input: &InstallerInput<'_>,
) -> Result<(Vec<InstallerReport>, Vec<PathBuf>), String> {
    let meta = Meta::new(input);
    let dir = input.out.join("installer");
    std::fs::create_dir_all(&dir).map_err(io("create installer directory"))?;
    let mut reports = Vec::new();
    let mut artifacts = Vec::new();
    let mut seen = Vec::new();
    for &format in &input.installer.formats {
        if !formats_for(input.platform).contains(&format) || seen.contains(&format) {
            continue;
        }
        seen.push(format);
        let staging = dir.join("staging").join(format.as_str());
        let _ = std::fs::remove_dir_all(&staging);
        let outcome = match format {
            ArtifactFormat::Dmg => dmg::build(input, &meta, &dir, &staging),
            ArtifactFormat::Msi => wix::build(input, &meta, &dir, &staging),
            ArtifactFormat::Nsis => nsis::build(input, &meta, &dir, &staging),
            ArtifactFormat::Deb => deb::build(input, &meta, &dir, &staging),
            ArtifactFormat::Appimage => linux_bundles::appimage(input, &meta, &dir, &staging),
            ArtifactFormat::Flatpak => linux_bundles::flatpak(input, &meta, &dir, &staging),
            ArtifactFormat::Archive => unreachable!("rejected by the config check"),
        }?;
        let relative = |path: &Path| {
            path.strip_prefix(input.out)
                .unwrap_or(path)
                .display()
                .to_string()
        };
        let status = match outcome {
            Outcome::Built { path, checks } => {
                let _ = std::fs::remove_dir_all(&staging);
                let bytes = std::fs::read(&path).map_err(io("read built installer"))?;
                let status = InstallerStatus::Built {
                    file: relative(&path),
                    size: bytes.len() as u64,
                    blake3: content_hex(&bytes),
                    checks,
                };
                artifacts.push(path);
                status
            }
            Outcome::NotExecuted { reason, staged } => InstallerStatus::NotExecuted {
                reason,
                staged: staged.as_deref().map(relative),
            },
        };
        reports.push(InstallerReport {
            format: format.as_str(),
            status,
        });
    }
    let staging = dir.join("staging");
    if staging.is_dir()
        && std::fs::read_dir(&staging).is_ok_and(|mut entries| entries.next().is_none())
    {
        let _ = std::fs::remove_dir(&staging);
    }
    Ok((reports, artifacts))
}

/// The architecture name a triple's first component maps to, by table.
pub(crate) fn arch<'a>(triple: &str, table: &[(&str, &'a str)]) -> Result<&'a str, String> {
    let first = triple.split('-').next().unwrap_or_default();
    table
        .iter()
        .find(|(prefix, _)| {
            if let Some(stem) = prefix.strip_suffix('*') {
                first.starts_with(stem)
            } else {
                first == *prefix
            }
        })
        .map(|(_, name)| *name)
        .ok_or_else(|| format!("no installer architecture for target `{triple}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_package_names_derive_from_the_application_name() {
        assert_eq!(
            derive_linux_package("Nana Fixture", "NanaFixture"),
            "nana-fixture"
        );
        assert_eq!(derive_linux_package("ナナ", "NanaLive"), "nanalive");
        assert_eq!(derive_linux_package("!", "?"), "nana-app");
        assert!(valid_linux_package("nana-fixture"));
        assert!(!valid_linux_package("Nana"));
        assert!(!valid_linux_package("-x"));
    }

    #[test]
    fn desktop_entries_quote_awkward_executables() {
        let meta = Meta {
            id: "dev.nana.fixture".into(),
            name: "Nana Fixture".into(),
            version: "1.0.0".into(),
            vendor: "Nana".into(),
            summary: "A fixture".into(),
            description: None,
            homepage: None,
            maintainer: "Nana".into(),
            package: "nana-fixture".into(),
            exe: "Nana Fixture".into(),
            stem: "Nana-Fixture".into(),
            categories: vec!["Utility".into(), "Development".into()],
        };
        let entry = desktop_entry(&meta);
        assert!(entry.contains("Exec=\"Nana Fixture\"\n"), "{entry}");
        assert!(entry.contains("Icon=dev.nana.fixture\n"));
        assert!(entry.contains("Categories=Utility;Development;\n"));
    }
}
