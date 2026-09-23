//! `nana-packager package`: one built executable + resources + config →
//! a distributable application directory, its manifest, and backend
//! metadata.
//!
//! Output directory:
//!
//! ```text
//! <out>/
//! ├─ app/                 the application (Windows/Linux: App.exe + runtime/;
//! │                       macOS: Name.app)
//! ├─ steam/               Steam backend: depot build scripts and file list
//! └─ package-report.json  what was built, reused, signed and skipped
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nana_package::PublisherKey;
use nana_package::manifest::{
    DistributionBackend, FileKind, ManifestApplication, ManifestBuild, ManifestDistribution,
    ManifestFile, ManifestNana, ManifestPack, ManifestPlugin, ManifestTarget, PackageManifest,
    PlatformSigning, RootException, SignStatus, SigningReport, StaticAppPlanStatus,
};
use serde::Serialize;

use crate::cache::ArtifactCache;
use crate::config::{Codec, MacosConfig, PackageConfig};
use crate::delta::{self, Delta, STEAM_CHUNK};
use crate::layout::{PackageLayout, TargetPlatform};
use crate::pack_build::{self, EntrySource, InputEntry, PackInput, PackReport};
use crate::plan::{self, PackPlan};
use crate::secrets::{self, LoadedSecrets, SecretSources};
use crate::sign::{self, SignContext};
use crate::util::{contains, content_hex, io, relative_path, walk_files};

/// Overflow-check panic text. A dev-profile binary always carries it; a
/// release binary usually does not, but can (core's shared panic strings
/// survive a non-LTO link on some targets), so it only raises a warning.
const DEBUG_ASSERTIONS_MARKER: &[u8] = b"attempt to add with overflow";
/// Reserved for the future self-update helper (workstream G). A Steam or
/// installer package must not contain it.
pub const UPDATER_MARKER: &[u8] = b"NANA-UPDATER-V1";
pub const REPORT_FILE: &str = "package-report.json";

#[derive(Debug, Clone)]
pub struct PackageOptions {
    pub config: PathBuf,
    pub executable: PathBuf,
    pub target: String,
    pub out: PathBuf,
    /// Output directory of the previous package (its `app/`), whose packs
    /// provide layout and bytes for a bounded Steam delta.
    pub baseline: Option<PathBuf>,
    pub cache: Option<PathBuf>,
    pub profile: String,
    pub build_id: Option<String>,
    pub compact: bool,
    pub secrets: SecretSources,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageReport {
    pub application: String,
    pub version: String,
    pub target: String,
    pub backend: String,
    pub app_root: String,
    pub signed: bool,
    pub packs: Vec<PackReport>,
    /// Per pack, compared with the baseline.
    pub deltas: BTreeMap<String, Delta>,
    pub platform_signing: Vec<nana_package::manifest::PlatformSigning>,
    pub warnings: Vec<String>,
}

pub fn package(options: &PackageOptions) -> Result<PackageReport, String> {
    let mut prepared = false;
    let result = package_into(options, &mut prepared);
    if result.is_err() && prepared {
        // A failed run leaves no half-built application (which could carry
        // a secret the leak scan never got to see) and no report.
        let _ = std::fs::remove_dir_all(options.out.join("app"));
        let _ = std::fs::remove_dir_all(options.out.join("steam"));
        let _ = std::fs::remove_file(options.out.join(REPORT_FILE));
    }
    result
}

fn package_into(options: &PackageOptions, prepared: &mut bool) -> Result<PackageReport, String> {
    let (config, base) = PackageConfig::load(&options.config)?;
    let platform = TargetPlatform::from_triple(&options.target)?;
    let pack_plan = PackPlan::build(&config, &base).map_err(|error| error.to_string())?;
    if let Some(pack_plan) = &pack_plan {
        plan::check_prefixes(pack_plan)?;
    }
    let mut warnings = Vec::new();

    // The binary is the one the config describes, and a release build.
    let exe_bytes = std::fs::read(&options.executable)
        .map_err(|error| format!("cannot read {}: {error}", options.executable.display()))?;
    check_identity(&config, &exe_bytes)?;
    let debug_assertions = contains(&exe_bytes, DEBUG_ASSERTIONS_MARKER);
    if debug_assertions {
        warnings.push(format!(
            "{} may be a dev-profile build (it carries overflow-check panic text); \
             distribution builds use --profile dist",
            options.executable.display()
        ));
    }
    if options.profile != "dist" {
        warnings.push(format!(
            "profile `{}` is not `dist`; distribution builds use --profile dist",
            options.profile
        ));
    }

    // Secrets: only what the plan needs.
    let mut needed = BTreeMap::new();
    if let Some(pack_plan) = &pack_plan {
        for pack in &pack_plan.packs {
            if let Some((name, key)) = &pack.key {
                needed.insert(name.clone(), key.generation);
            }
        }
    }
    let publisher_config = config.signing.publisher.as_ref();
    let key_supplied = options.secrets.signing_key_file.is_some()
        || std::env::var_os(secrets::PUBLISHER_ENV).is_some();
    let want_publisher = publisher_config.is_some_and(|p| p.required)
        || (publisher_config.is_some() && key_supplied);
    let repo_root = secrets::repository_root(&base);
    let loaded = LoadedSecrets::load(
        &needed,
        want_publisher,
        &options.secrets,
        repo_root.as_deref(),
    )?;
    if let (Some(config), Some((_, actual))) = (publisher_config, &loaded.publisher) {
        match &config.public_key {
            Some(text) => {
                let expected = PublisherKey::from_text(text)
                    .ok_or("signing.publisher.public_key must be `ed25519:<64 hex>`")?;
                if &expected != actual {
                    return Err(
                        "the publisher signing key does not match signing.publisher.public_key"
                            .into(),
                    );
                }
            }
            None => warnings.push(format!(
                "signing.publisher.public_key is not pinned; signed with {}",
                actual.to_text()
            )),
        }
    } else if publisher_config.is_none() && key_supplied {
        warnings.push(
            "a publisher signing key was supplied but nana-package.toml has no \
             [signing.publisher]: the build is unsigned"
                .into(),
        );
    } else if publisher_config.is_some() {
        warnings.push(
            "publisher signature not required and no signing key given: unsigned build".into(),
        );
    }
    let signer = loaded.publisher.as_ref().map(|(key, _)| key);

    // Fresh output directory.
    if let Some(cache) = &options.cache
        && resolve_path(cache).starts_with(resolve_path(&options.out))
    {
        return Err("--cache inside --out would be wiped by every run; keep it elsewhere".into());
    }
    prepare_out(&options.out, options.baseline.as_deref())?;
    *prepared = true;
    let layout = PackageLayout::new(
        platform,
        &config.application.name,
        &config.executable.file_name,
    );
    let app_parent = options.out.join("app");
    let app_root = layout.app_root(&app_parent);
    let at = |relative: &str| PackageLayout::join(&app_root, relative);
    for dir in [
        &layout.runtime_bin,
        &layout.runtime_resources,
        &layout.runtime_plugins,
        &layout.runtime_tools,
        &layout.runtime_manifest,
    ] {
        std::fs::create_dir_all(at(dir)).map_err(io("create layout"))?;
    }

    // Executable.
    let exe_path = at(&layout.executable);
    std::fs::create_dir_all(exe_path.parent().expect("executable has a parent"))
        .map_err(io("create executable directory"))?;
    std::fs::write(&exe_path, &exe_bytes).map_err(io("copy executable"))?;
    set_executable(&exe_path)?;
    let default_macos = MacosConfig::default();
    let macos = config.platform.macos.as_ref().unwrap_or(&default_macos);
    if platform == TargetPlatform::Macos && macos.strip {
        strip_macos(&exe_path, &mut warnings)?;
    }

    // Runtime files and plugins.
    let mut plugins = Vec::new();
    let copy_runtime =
        |files: &[crate::config::RuntimeFileConfig], dir: &str| -> Result<(), String> {
            for file in files {
                if !ships_on(&file.platforms, platform) {
                    continue;
                }
                copy_into(&base.join(&file.source), &at(dir))?;
            }
            Ok(())
        };
    copy_runtime(&config.runtime.bin, &layout.runtime_bin)?;
    copy_runtime(&config.runtime.tools, &layout.runtime_tools)?;
    for plugin in &config.runtime.plugins {
        if !ships_on(&plugin.platforms, platform) {
            continue;
        }
        let file = copy_into(&base.join(&plugin.source), &at(&layout.runtime_plugins))?;
        plugins.push(ManifestPlugin {
            name: plugin.name.clone(),
            file,
            abi: plugin.abi.clone(),
            version: plugin.version.clone(),
        });
    }
    let mut root_exceptions = Vec::new();
    if platform == TargetPlatform::Windows
        && let Some(windows) = &config.platform.windows
    {
        for exception in &windows.root_exceptions {
            if exception.reason.trim().is_empty() {
                return Err("platform.windows.root_exceptions entries need a reason".into());
            }
            let file = copy_into(&base.join(&exception.source), &app_root)?;
            root_exceptions.push(RootException {
                file,
                reason: exception.reason.clone(),
            });
        }
    }

    // Resource packs.
    let cache = options.cache.as_deref().map(ArtifactCache::new);
    let mut pack_reports = Vec::new();
    let mut manifest_packs = Vec::new();
    let mut deltas = BTreeMap::new();
    let baseline_app = options
        .baseline
        .as_ref()
        .map(|dir| layout.app_root(&dir.join("app")));
    if let Some(pack_plan) = &pack_plan {
        for pack in &pack_plan.packs {
            let file = format!("{}.nrpack", pack.name);
            let out_path = at(&layout.runtime_resources).join(&file);
            let baseline_path = baseline_app
                .as_ref()
                .map(|root| PackageLayout::join(root, &layout.runtime_resources).join(&file));
            let encryption = pack
                .key
                .as_ref()
                .map(|(name, _)| loaded.content[name].clone());
            let built = pack_build::build_pack(
                PackInput {
                    application_id: &config.application.id,
                    name: &pack.name,
                    class: pack.class,
                    entries: pack
                        .entries
                        .iter()
                        .map(|entry| InputEntry {
                            key: entry.key.clone(),
                            source: EntrySource::File(entry.source.clone()),
                        })
                        .collect(),
                    block_size: pack_plan.block_size,
                    zstd_level: (pack.compression.codec == Codec::Zstd)
                        .then_some(pack.compression.level),
                    encryption: encryption.clone(),
                    signer,
                    baseline: baseline_path.as_deref(),
                    cache: cache.as_ref(),
                    compact: options.compact,
                    max_free_ratio: pack_plan.max_free_ratio,
                },
                &out_path,
            )?;
            if let Some(limit) = pack.max_bytes
                && built.report.pack_bytes > limit
            {
                return Err(format!(
                    "pack `{}` is {} bytes, over its max_bytes {limit}; split it (large packs \
                     cost Steam update IO)",
                    pack.name, built.report.pack_bytes
                ));
            }
            if let Some(baseline_path) = &baseline_path
                && let Ok(old) = std::fs::read(baseline_path)
            {
                let new = std::fs::read(&out_path).map_err(io("read built pack"))?;
                deltas.insert(pack.name.clone(), delta::delta(&old, &new, STEAM_CHUNK));
            }
            let header = &built.header;
            manifest_packs.push(ManifestPack {
                name: pack.name.clone(),
                file,
                class: pack.class,
                prefixes: pack.prefixes.clone(),
                format_version: header.version,
                pack_id: nana_package::to_hex(&header.pack_id),
                toc_hash: nana_package::to_hex(&header.toc_hash),
                size: built.report.pack_bytes,
                entries: header.entry_count,
                key_name: pack.key.as_ref().map(|(name, _)| name.clone()),
                key_id: encryption.as_ref().map(|(id, _, _)| id.to_string()),
                key_generation: header.key_generation,
                signed: header.signed(),
                depends_on: pack.depends_on.clone(),
            });
            pack_reports.push(built.report);
        }
    }

    // Platform metadata.
    if platform == TargetPlatform::Macos {
        let extra = match &macos.info_plist_extra {
            Some(path) => Some(
                std::fs::read_to_string(base.join(path))
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?,
            ),
            None => None,
        };
        let plist = crate::macos::info_plist(&crate::macos::PlistInput {
            application: &config.application,
            macos,
            executable: &config.executable.file_name,
            build_id: options.build_id.as_deref(),
            extra: extra.as_deref(),
        })?;
        std::fs::write(app_root.join("Contents/Info.plist"), plist)
            .map_err(io("write Info.plist"))?;
        let icon = match &macos.icon {
            Some(path) => std::fs::read(base.join(path))
                .map_err(|error| format!("cannot read icon {}: {error}", path.display()))?,
            None => nana_app_icon::encode_icns()?,
        };
        std::fs::write(at(&layout.runtime_resources).join("AppIcon.icns"), icon)
            .map_err(io("write AppIcon.icns"))?;
    }
    if config.distribution.backend == DistributionBackend::Portable
        && platform != TargetPlatform::Macos
    {
        std::fs::write(at(&layout.runtime_manifest).join("portable"), b"")
            .map_err(io("write portable marker"))?;
    }

    // Manifest. Platform signing runs after it: codesign seals the bundle,
    // manifest included, and Authenticode / codesign rewrite executables, so
    // their hashes here are the pre-signing ones and the manifest can only
    // say which adapters are configured, not how they ended (the report
    // does).
    let manifest_dir = at(&layout.runtime_manifest);
    let runtime_files = list_files(&app_root, &layout, &manifest_dir)?;
    let configured_signing = sign::configured(&config.signing.platform, platform)?
        .into_iter()
        .map(|adapter| PlatformSigning {
            adapter: adapter.to_owned(),
            status: SignStatus::NotExecuted {
                reason: "platform signing runs after the manifest is written; see \
                         package-report.json and verify with the platform tool"
                    .into(),
            },
        })
        .collect();
    let manifest = PackageManifest {
        schema: nana_package::MANIFEST_SCHEMA.into(),
        schema_version: nana_package::MANIFEST_SCHEMA_VERSION,
        application: ManifestApplication {
            id: config.application.id.clone(),
            name: config.application.name.clone(),
            version: config.application.version.clone(),
            vendor: config.application.vendor.clone(),
            build_id: options.build_id.clone(),
        },
        target: ManifestTarget {
            triple: options.target.clone(),
            platform: platform.as_str().into(),
        },
        nana: ManifestNana {
            version: nana_package::NANA_VERSION.into(),
        },
        build: ManifestBuild {
            profile: options.profile.clone(),
            debug_assertions_suspected: debug_assertions,
        },
        static_app_plan: StaticAppPlanStatus::Unavailable {
            reason: "StaticAppPlan is not produced yet (Issue #158)".into(),
        },
        distribution: ManifestDistribution {
            backend: config.distribution.backend,
            self_update: false,
        },
        layout: layout.manifest_layout(root_exceptions),
        resource_packs: manifest_packs,
        plugins,
        runtime_files,
        signing: SigningReport {
            publisher: if loaded.publisher.is_some() {
                SignStatus::Signed
            } else {
                SignStatus::NotConfigured
            },
            publisher_key: loaded
                .publisher
                .as_ref()
                .map(|(_, public)| public.to_text()),
            platform: configured_signing,
        },
    };
    let json = manifest.to_json();
    // What the application will parse at startup: refuse to ship a manifest
    // its own runtime would reject.
    PackageManifest::from_json(json.as_bytes()).map_err(|error| {
        format!("the generated package manifest would be rejected at run time: {error}")
    })?;
    std::fs::write(manifest_dir.join(nana_package::MANIFEST_FILE), &json)
        .map_err(io("write manifest"))?;
    if let Some((signing, _)) = &loaded.publisher {
        use ed25519_dalek::Signer;
        let signature = signing.sign(&PackageManifest::signing_message(json.as_bytes()));
        std::fs::write(
            manifest_dir.join(nana_package::MANIFEST_SIGNATURE_FILE),
            signature.to_bytes(),
        )
        .map_err(io("write manifest signature"))?;
    }

    let platform_signing = sign::run_platform_signing(
        &config.signing.platform,
        &SignContext {
            platform,
            app_root: &app_root,
            executable: &exe_path,
        },
    )?;

    if config.distribution.backend == DistributionBackend::Steam {
        crate::steam::write_steam_output(&config, platform, &options.out, &app_parent, &layout)?;
    }

    // Nothing that carries a secret leaves this function: on a hit, the
    // application and backend output are removed and no report is written.
    let leaks = secrets::scan_for_leaks(&options.out, &loaded)?;
    if !leaks.is_empty() {
        return Err(format!(
            "secret material found in the output ({}); the output was deleted",
            leaks.join(", ")
        ));
    }

    let report = PackageReport {
        application: config.application.id.clone(),
        version: config.application.version.clone(),
        target: options.target.clone(),
        backend: config.distribution.backend.as_str().into(),
        app_root: app_root
            .strip_prefix(&options.out)
            .unwrap_or(&app_root)
            .display()
            .to_string(),
        signed: loaded.publisher.is_some(),
        packs: pack_reports,
        deltas,
        platform_signing,
        warnings,
    };
    std::fs::write(
        options.out.join(REPORT_FILE),
        serde_json::to_string_pretty(&report).expect("report serializes"),
    )
    .map_err(io("write package report"))?;
    Ok(report)
}

fn check_identity(config: &PackageConfig, exe: &[u8]) -> Result<(), String> {
    let embedded = nana_package::find_marker(exe).map_err(|error| error.to_string())?;
    let app = &config.application;
    let vendor = app.vendor.clone().unwrap_or_default();
    let declared = nana_package::manifest::ManifestApplication {
        id: app.id.clone(),
        name: app.name.clone(),
        version: app.version.clone(),
        vendor: app.vendor.clone(),
        build_id: None,
    };
    if !declared.matches(
        &embedded.id,
        &embedded.name,
        &embedded.version,
        Some(&embedded.vendor),
    ) {
        return Err(format!(
            "the executable declares {} \"{}\" {} (vendor \"{}\") but nana-package.toml says {} \
             \"{}\" {} (vendor \"{}\")",
            embedded.id,
            embedded.name,
            embedded.version,
            embedded.vendor,
            app.id,
            app.name,
            app.version,
            vendor
        ));
    }
    Ok(())
}

/// `path` with its nearest existing ancestor canonicalized (symlinks
/// resolved) and the rest appended with `.` / `..` applied, so paths that do
/// not exist yet compare correctly against ones that do.
fn resolve_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut normal = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    let mut existing = normal.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(existing) {
            return rest
                .iter()
                .rev()
                .fold(canonical, |dir, part| dir.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => return normal,
        }
    }
}

/// Marks a directory `nana-packager` created, so a later run may clear it
/// even when the previous run failed half way.
pub const OUT_MARKER: &str = ".nana-packager-out";

/// Create `out`, refusing to wipe a directory this tool did not produce.
fn prepare_out(out: &Path, baseline: Option<&Path>) -> Result<(), String> {
    if let Some(baseline) = baseline
        && let (Ok(a), Ok(b)) = (std::fs::canonicalize(out), std::fs::canonicalize(baseline))
        && (a == b || b.starts_with(&a) || a.starts_with(&b))
    {
        return Err("--baseline and --out must be separate directories".into());
    }
    if out.exists() {
        let empty = std::fs::read_dir(out)
            .map_err(io("read --out"))?
            .next()
            .is_none();
        if !empty && !out.join(OUT_MARKER).is_file() {
            return Err(format!(
                "{} exists and was not produced by nana-packager; choose an empty directory",
                out.display()
            ));
        }
        std::fs::remove_dir_all(out).map_err(io("clear --out"))?;
    }
    std::fs::create_dir_all(out).map_err(io("create --out"))?;
    std::fs::write(out.join(OUT_MARKER), b"").map_err(io("mark --out"))
}

fn copy_into(source: &Path, dir: &Path) -> Result<String, String> {
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{} has no UTF-8 file name", source.display()))?;
    std::fs::create_dir_all(dir).map_err(io("create runtime directory"))?;
    let dest = dir.join(name);
    if dest.exists() {
        return Err(format!("two runtime files are named {name}"));
    }
    std::fs::copy(source, &dest)
        .map_err(|error| format!("cannot copy {}: {error}", source.display()))?;
    Ok(name.to_owned())
}

/// A runtime file ships on every platform unless it names some.
fn ships_on(platforms: &[String], platform: TargetPlatform) -> bool {
    platforms.is_empty() || platforms.iter().any(|p| p == platform.as_str())
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(io("chmod executable"))?;
    }
    let _ = path;
    Ok(())
}

/// `strip -x` drops local symbols and keeps the dynamic table; a release
/// build still carries its full symbol table otherwise.
fn strip_macos(exe: &Path, warnings: &mut Vec<String>) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        warnings.push("strip skipped: macOS strip is only available on a macOS host".into());
        return Ok(());
    }
    let status = std::process::Command::new("strip")
        .arg("-x")
        .arg(exe)
        .status()
        .map_err(|error| format!("cannot run strip: {error}"))?;
    if !status.success() {
        return Err(format!("strip failed on {}: {status}", exe.display()));
    }
    Ok(())
}

/// Every file of the package except the manifest itself, sorted.
fn list_files(
    app_root: &Path,
    layout: &PackageLayout,
    manifest_dir: &Path,
) -> Result<Vec<ManifestFile>, String> {
    let mut files = Vec::new();
    for path in walk_files(app_root)? {
        if path.parent() == Some(manifest_dir) {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == nana_package::MANIFEST_FILE || name == nana_package::MANIFEST_SIGNATURE_FILE
            {
                continue;
            }
        }
        let relative = relative_path(app_root, &path)?;
        let bytes = std::fs::read(&path).map_err(io("hash package file"))?;
        let under = |dir: &str| relative.starts_with(&format!("{dir}/"));
        let kind = if relative == layout.executable {
            FileKind::Executable
        } else if under(&layout.runtime_manifest) {
            FileKind::Metadata
        } else if under(&layout.runtime_bin) {
            FileKind::Library
        } else if under(&layout.runtime_tools) {
            FileKind::Tool
        } else if under(&layout.runtime_plugins) {
            FileKind::Plugin
        } else if under(&layout.runtime_resources) {
            FileKind::Resource
        } else if relative.ends_with(".dll")
            || relative.ends_with(".so")
            || relative.ends_with(".dylib")
        {
            FileKind::Library
        } else {
            FileKind::Metadata
        };
        let signable = matches!(
            kind,
            FileKind::Executable | FileKind::Library | FileKind::Tool | FileKind::Plugin
        );
        files.push(ManifestFile {
            path: relative,
            kind,
            size: bytes.len() as u64,
            blake3: content_hex(&bytes),
            pre_platform_signing: signable,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_path_matches_existing_and_future_paths() {
        let base = std::env::temp_dir().join(format!("nana-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let existing = resolve_path(&base);
        // Not created yet, reached through `..`: still inside `base`.
        let future = resolve_path(&base.join("x/../cache/deeper"));
        assert!(future.starts_with(&existing), "{future:?} vs {existing:?}");
        assert!(!resolve_path(&base.join("../elsewhere")).starts_with(&existing));
        let _ = std::fs::remove_dir_all(&base);
    }
}
