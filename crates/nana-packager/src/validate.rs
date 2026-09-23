//! Final-artifact validation (Issue #226 §12): checks run against the
//! delivered application directory, not against `target/`.
//!
//! Every check reports `pass`, `fail`, `warn`, or `not-executed` with the
//! reason; a check that could not run is never reported as passing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nana_package::manifest::{DistributionBackend, PackageManifest, SignStatus, SignatureState};
use nana_package::{
    ContentKey, KeyId, PackReader, PublisherKey, ReadStats, StaticKeys, TrustPolicy,
};
use serde::Serialize;

use crate::layout::{PackageLayout, TargetPlatform};
use crate::package::UPDATER_MARKER;
use crate::secrets::content_key_env;
use crate::util::{contains, content_hex, relative_path, walk_files};
use nana_package::pack::format;

use nana_package::{SELF_CHECK_ENV, SELF_CHECK_PREFIX};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "detail", rename_all = "kebab-case")]
pub enum Status {
    Pass,
    Warn(String),
    Fail(String),
    NotExecuted(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    #[serde(flatten)]
    pub status: Status,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub app_root: String,
    pub checks: Vec<Check>,
}

impl ValidationReport {
    pub fn failed(&self) -> bool {
        self.checks
            .iter()
            .any(|c| matches!(c.status, Status::Fail(_)))
    }

    fn push(&mut self, name: impl Into<String>, status: Status) {
        self.checks.push(Check {
            name: name.into(),
            status,
        });
    }
}

#[derive(Debug, Clone, Default)]
pub struct ValidateOptions {
    /// `App/` (holding App.exe and runtime/) or `Name.app`.
    pub app_root: PathBuf,
    /// Pinned publisher key; without it signatures are reported not checked.
    pub trust: Option<PublisherKey>,
    /// Launch the packaged executable in self-check mode.
    pub run: bool,
    /// Tamper with copies of the package and expect each launch to fail.
    pub tamper_suite: bool,
    /// Extra environment for launches (the application's own key source).
    pub run_env: Vec<(String, String)>,
    /// Executables and libraries were platform-signed after packaging, which
    /// rewrites them: report their hash change as a warning, not a failure.
    /// Verify the signatures with the platform tool (`codesign --verify`,
    /// `signtool verify`); this validator does not.
    pub allow_resigned: bool,
    /// Tamper case `key-missing`: relaunch without these environment
    /// variables (the application's key source) and expect a failure.
    pub tamper_drop_env: Vec<String>,
}

struct Located {
    platform: TargetPlatform,
    layout: PackageLayout,
    manifest_dir: PathBuf,
}

/// The manifest directory of `app_root`: inside a `.app` bundle, or
/// `runtime/manifest` next to the executable.
fn manifest_dir(app_root: &Path) -> Result<PathBuf, String> {
    let is_bundle = app_root.extension().is_some_and(|ext| ext == "app");
    let dir = if is_bundle {
        "Contents/Resources/manifest"
    } else {
        "runtime/manifest"
    };
    let manifest_dir = PackageLayout::join(app_root, dir);
    if manifest_dir.join(nana_package::MANIFEST_FILE).is_file() {
        Ok(manifest_dir)
    } else {
        Err(format!(
            "{} has no package manifest (runtime/manifest/package.json or \
             Contents/Resources/manifest/package.json)",
            app_root.display()
        ))
    }
}

pub fn validate(options: &ValidateOptions) -> Result<ValidationReport, String> {
    let app_root = &options.app_root;
    let mut report = ValidationReport {
        app_root: app_root.display().to_string(),
        checks: Vec::new(),
    };
    let manifest_dir = manifest_dir(app_root)?;
    let trust = match &options.trust {
        Some(key) => TrustPolicy::RequirePublisher(key.clone()),
        None => TrustPolicy::AllowUnsigned,
    };

    // Manifest and its signature.
    let (manifest, manifest_verified) = match PackageManifest::read(&manifest_dir, &trust) {
        Ok((manifest, state)) => {
            report.push(
                "manifest.signature",
                match state {
                    SignatureState::Verified => Status::Pass,
                    SignatureState::Unsigned => Status::Warn("unsigned build".into()),
                    SignatureState::NotChecked => {
                        Status::NotExecuted("signed, but no --trust-key to check it against".into())
                    }
                },
            );
            (manifest, state == SignatureState::Verified)
        }
        Err(error) => {
            report.push("manifest.read", Status::Fail(error.to_string()));
            return Ok(report);
        }
    };
    let platform = TargetPlatform::parse(&manifest.target.platform).ok_or_else(|| {
        format!(
            "manifest names an unknown platform `{}`",
            manifest.target.platform
        )
    })?;
    let root_name = app_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let located = Located {
        platform,
        layout: PackageLayout::from_manifest(root_name, &manifest.layout),
        manifest_dir,
    };
    let layout = &located.layout;
    let exe_path = PackageLayout::join(app_root, &layout.executable);

    check_root(&mut report, app_root, located.platform, &manifest);
    check_files(
        &mut report,
        app_root,
        &located,
        &manifest,
        options.allow_resigned,
    )?;
    check_identity(&mut report, &exe_path, &manifest);
    check_imports(&mut report, app_root, located.platform, layout, &exe_path);
    check_packs(&mut report, app_root, layout, &manifest, &trust);
    check_distribution(&mut report, app_root, &manifest)?;

    // What the manifest claims about signing is not evidence: the publisher
    // signature passes only when it was checked against a pinned key here
    // (manifest and packs opened under `RequirePublisher`), and platform
    // signatures only when the adapter verifies them now.
    report.push(
        "signing.publisher",
        if manifest_verified {
            Status::Pass
        } else if manifest.signing.publisher.is_signed() {
            Status::NotExecuted("signed, but no --trust-key to verify it against".into())
        } else {
            Status::Warn("resource packs and manifest are not publisher-signed".into())
        },
    );
    for platform in &manifest.signing.platform {
        let context = crate::sign::SignContext {
            platform: located.platform,
            app_root,
            executable: &exe_path,
        };
        let status = match crate::sign::adapter(&platform.adapter) {
            Some(adapter) => match adapter.verify(&context) {
                SignStatus::Verified | SignStatus::Signed => Status::Pass,
                SignStatus::Failed { detail } => Status::Fail(detail),
                SignStatus::NotExecuted { reason } => Status::NotExecuted(reason),
                SignStatus::NotConfigured => Status::NotExecuted("not configured".into()),
            },
            None => Status::Fail(format!("unknown signing adapter `{}`", platform.adapter)),
        };
        report.push(format!("signing.{}", platform.adapter), status);
    }

    let host = TargetPlatform::from_triple(&TargetPlatform::host_triple()).ok();
    let runnable = host == Some(located.platform);
    if options.run {
        if runnable {
            let status = match self_check(&exe_path, &options.run_env, &[]) {
                Ok(check) if check.passed() => Status::Pass,
                Ok(check) => Status::Fail(check.describe()),
                Err(error) => Status::Fail(error),
            };
            report.push("run.launch-from-foreign-cwd", status);
        } else {
            report.push(
                "run.launch-from-foreign-cwd",
                Status::NotExecuted(format!(
                    "{} package on a {} host",
                    located.platform.as_str(),
                    host.map_or("unknown", TargetPlatform::as_str)
                )),
            );
        }
    }
    if options.tamper_suite {
        if runnable {
            tamper_suite(
                &mut report,
                app_root,
                &located,
                &manifest,
                &options.run_env,
                &options.tamper_drop_env,
            )?;
        } else {
            report.push(
                "tamper-suite",
                Status::NotExecuted("package is not for this host".into()),
            );
        }
    }

    for (name, reason) in [
        (
            "startup.early-splash",
            "the self-check exits before any window, so it cannot see a native splash; \
             verify on a real window with `startup-splash --probe` (Issue #225); \
             the logo is not read from the early-splash pack yet",
        ),
        (
            "startup.ui-ready-handoff",
            "the first-frame handoff needs a presented window; the self-check exits before \
             one exists (verify with `startup-splash --probe`, Issue #225)",
        ),
        (
            "installer.round-trip",
            "installer backends are not implemented yet (Issue #226 workstream F)",
        ),
        (
            "static-app-plan",
            "StaticAppPlan fingerprints are not produced yet (Issue #158)",
        ),
    ] {
        report.push(name, Status::NotExecuted(reason.into()));
    }
    Ok(report)
}

fn check_root(
    report: &mut ValidationReport,
    app_root: &Path,
    platform: TargetPlatform,
    manifest: &PackageManifest,
) {
    let exceptions: BTreeSet<&str> = manifest
        .layout
        .root_exceptions
        .iter()
        .map(|e| e.file.as_str())
        .collect();
    let mut problems = Vec::new();
    match platform {
        TargetPlatform::Macos => {
            let macos_dir = app_root.join("Contents/MacOS");
            let expected = Path::new(&manifest.layout.executable)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match std::fs::read_dir(&macos_dir) {
                Ok(items) => {
                    for item in items.flatten() {
                        let name = item.file_name().to_string_lossy().into_owned();
                        if name != expected {
                            problems.push(format!("Contents/MacOS/{name}"));
                        }
                    }
                }
                Err(error) => problems.push(format!("Contents/MacOS: {error}")),
            }
        }
        _ => match std::fs::read_dir(app_root) {
            Ok(items) => {
                for item in items.flatten() {
                    let name = item.file_name().to_string_lossy().into_owned();
                    let allowed = name == manifest.layout.executable
                        || name == "runtime"
                        || exceptions.contains(name.as_str())
                        // A portable build writes its data next to the executable.
                        || (name == "data" && manifest.distribution.backend == DistributionBackend::Portable);
                    if !allowed {
                        problems.push(name);
                    }
                }
            }
            Err(error) => problems.push(error.to_string()),
        },
    }
    report.push(
        "layout.single-root-entry",
        if problems.is_empty() {
            Status::Pass
        } else {
            let libraries = problems.iter().any(|p| {
                platform
                    .library_extensions()
                    .iter()
                    .any(|ext| p.ends_with(&format!(".{ext}")))
            });
            Status::Fail(format!(
                "unexpected {}in the application root: {}",
                if libraries { "libraries " } else { "files " },
                problems.join(", ")
            ))
        },
    );
}

fn check_files(
    report: &mut ValidationReport,
    app_root: &Path,
    located: &Located,
    manifest: &PackageManifest,
    allow_resigned: bool,
) -> Result<(), String> {
    let mut listed = BTreeSet::new();
    let mut failures = Vec::new();
    let mut resigned = Vec::new();
    for file in &manifest.runtime_files {
        listed.insert(file.path.clone());
        let path = PackageLayout::join(app_root, &file.path);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let hash = content_hex(&bytes);
                if hash != file.blake3 || bytes.len() as u64 != file.size {
                    if file.pre_platform_signing && allow_resigned {
                        resigned.push(file.path.clone());
                    } else {
                        failures.push(format!("{} changed", file.path));
                    }
                }
            }
            Err(_) => failures.push(format!("{} is missing", file.path)),
        }
    }
    // Files the manifest does not list.
    for path in walk_files(app_root)? {
        let relative = relative_path(app_root, &path)?;
        let in_manifest_dir = path.parent() == Some(located.manifest_dir.as_path());
        let manifest_file = in_manifest_dir
            && [
                nana_package::MANIFEST_FILE,
                nana_package::MANIFEST_SIGNATURE_FILE,
            ]
            .iter()
            .any(|name| path.file_name().is_some_and(|n| n == *name));
        let data = relative.starts_with("data/")
            && manifest.distribution.backend == DistributionBackend::Portable;
        // Code signatures added after packaging.
        let signature = relative.starts_with("Contents/_CodeSignature/")
            || relative == "Contents/CodeResources";
        if !listed.contains(&relative) && !manifest_file && !data && !signature {
            failures.push(format!("{relative} is not in the manifest"));
        }
    }
    report.push(
        "files.manifest-match",
        if failures.is_empty() {
            Status::Pass
        } else {
            Status::Fail(failures.join("; "))
        },
    );
    if !resigned.is_empty() {
        report.push(
            "files.platform-signed-after-packaging",
            Status::Warn(format!(
                "changed since packaging, accepted as platform-signed (--allow-resigned; verify \
                 the signatures with the platform tool): {}",
                resigned.join(", ")
            )),
        );
    }
    Ok(())
}

fn check_identity(report: &mut ValidationReport, exe: &Path, manifest: &PackageManifest) {
    let status = match std::fs::read(exe) {
        Err(error) => Status::Fail(format!("cannot read executable: {error}")),
        Ok(bytes) => match nana_package::find_marker(&bytes) {
            Err(error) => Status::Fail(error.to_string()),
            Ok(embedded) => {
                let app = &manifest.application;
                if app.matches(
                    &embedded.id,
                    &embedded.name,
                    &embedded.version,
                    Some(&embedded.vendor),
                ) {
                    Status::Pass
                } else {
                    Status::Fail(format!(
                        "binary declares {} {} but the manifest says {} {}",
                        embedded.id, embedded.version, app.id, app.version
                    ))
                }
            }
        },
    };
    report.push("identity.binary-matches-manifest", status);
}

/// Dynamic-library references of the executable must resolve through the
/// package layout: a library in `runtime/bin` cannot satisfy an implicit
/// Windows import (the loader only searches the executable's directory and
/// system paths), a macOS reference must stay inside the bundle, and a Linux
/// `NEEDED` shipped in `runtime/bin` needs `$ORIGIN/runtime/bin` in RUNPATH.
fn check_imports(
    report: &mut ValidationReport,
    app_root: &Path,
    platform: TargetPlatform,
    layout: &PackageLayout,
    exe: &Path,
) {
    let Ok(bytes) = std::fs::read(exe) else {
        return;
    };
    let file = match object::File::parse(&*bytes) {
        Ok(file) => file,
        Err(error) => {
            report.push(
                "dependencies.discoverable",
                Status::NotExecuted(format!("cannot parse the executable: {error}")),
            );
            return;
        }
    };
    let bin = PackageLayout::join(app_root, &layout.runtime_bin);
    let mut problems = Vec::new();
    let mut unshipped = Vec::new();
    let mut redistributable = Vec::new();
    let mut libraries: BTreeSet<String> = BTreeSet::new();
    if let Ok(imports) = object::Object::imports(&file) {
        for import in imports.flatten() {
            let library = String::from_utf8_lossy(import.library()).into_owned();
            if !library.is_empty() {
                libraries.insert(library);
            }
        }
    }
    match platform {
        TargetPlatform::Windows => {
            let in_root = lowercase_names(app_root);
            let in_bin = lowercase_names(&bin);
            for library in &libraries {
                let lower = library.to_ascii_lowercase();
                // The debug C runtime may not be redistributed, shipped or not.
                if is_debug_crt(&lower) {
                    problems.push(format!(
                        "{library} is the debug C runtime, which may not be redistributed: this \
                         is a debug build"
                    ));
                    continue;
                }
                if in_root.contains(&lower) {
                    continue;
                }
                if in_bin.contains(&lower) {
                    problems.push(format!(
                        "{library} is imported implicitly but ships in runtime/bin, where the \
                         Windows loader does not look; delay-load it and add runtime/bin with \
                         AddDllDirectory, or declare it as a root exception"
                    ));
                } else if lower.starts_with("vcruntime") || lower.starts_with("msvcp") {
                    redistributable.push(library.clone());
                } else if !is_windows_system_library(&lower) {
                    unshipped.push(library.clone());
                }
            }
        }
        TargetPlatform::Macos => {
            let (rpaths, dylibs) = macho_load_paths(&file);
            // dyld loads every LC_LOAD_DYLIB, bound symbols or not.
            libraries.extend(dylibs);
            let frameworks = [
                "@executable_path/../Frameworks",
                "@loader_path/../Frameworks",
            ];
            for library in &libraries {
                if library.starts_with("/usr/lib/") || library.starts_with("/System/") {
                    continue;
                }
                let resolved = if let Some(rest) = library.strip_prefix("@rpath/") {
                    if !rpaths
                        .iter()
                        .any(|r| frameworks.contains(&r.trim_end_matches('/')))
                    {
                        problems.push(format!(
                            "{library} needs an LC_RPATH of {} (link with -Wl,-rpath,{})",
                            frameworks[0], frameworks[0]
                        ));
                        continue;
                    }
                    Some(rest)
                } else if let Some(beside) = library
                    .strip_prefix("@executable_path/")
                    .or_else(|| library.strip_prefix("@loader_path/"))
                    .filter(|rest| !rest.starts_with("../"))
                {
                    // Next to the executable in Contents/MacOS: loadable,
                    // though Frameworks is the conventional place.
                    let macos = exe.parent().unwrap_or(app_root);
                    if !macos.join(beside).exists() {
                        problems.push(format!("{library} is not in Contents/MacOS"));
                    }
                    continue;
                } else {
                    library
                        .strip_prefix("@executable_path/../Frameworks/")
                        .or_else(|| library.strip_prefix("@loader_path/../Frameworks/"))
                };
                match resolved {
                    Some(rest) if bin.join(rest).exists() => {}
                    Some(_) => problems.push(format!("{library} is not in Contents/Frameworks")),
                    None => {
                        problems.push(format!("{library} is an absolute path outside the bundle"))
                    }
                }
            }
        }
        TargetPlatform::Linux => {
            let (needed, runpath) = elf_dynamic(&file);
            for library in needed {
                if bin.join(&library).is_file()
                    && !runpath.iter().flat_map(|p| p.split(':')).any(|entry| {
                        matches!(
                            entry.trim_end_matches('/'),
                            "$ORIGIN/runtime/bin" | "${ORIGIN}/runtime/bin"
                        )
                    })
                {
                    problems.push(format!(
                        "{library} ships in runtime/bin but RUNPATH lacks $ORIGIN/runtime/bin \
                         (link with -Wl,-rpath,'$ORIGIN/runtime/bin')"
                    ));
                }
            }
        }
    }
    report.push(
        "dependencies.discoverable",
        if problems.is_empty() {
            Status::Pass
        } else {
            Status::Fail(problems.join("; "))
        },
    );
    if !redistributable.is_empty() {
        report.push(
            "dependencies.vc-runtime",
            Status::Warn(format!(
                "links the VC++ runtime dynamically ({}); it is not part of Windows: build with \
                 `-C target-feature=+crt-static`, or install the redistributable (Steam \
                 common redist / installer prerequisite)",
                redistributable.join(", ")
            )),
        );
    }
    if !unshipped.is_empty() {
        report.push(
            "dependencies.assumed-system",
            Status::Warn(format!(
                "imported but not shipped, assumed to come from the system: {}",
                unshipped.join(", ")
            )),
        );
    }
}

/// File names in `dir`, lowercased (PE import names and file names often
/// differ in case; the Windows loader does not care).
fn lowercase_names(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|item| item.file_name().to_string_lossy().to_ascii_lowercase())
        .collect()
}

/// `vcruntime140d.dll`, `msvcp140d.dll`, `msvcp140d_atomic_wait.dll`,
/// `ucrtbased.dll`, ...
fn is_debug_crt(lower: &str) -> bool {
    let stem = lower.trim_end_matches(".dll");
    lower == "ucrtbased.dll"
        || ((stem.starts_with("vcruntime") || stem.starts_with("msvcp"))
            && (stem.ends_with('d') || stem.contains("d_")))
}

/// DLLs every supported Windows provides. Anything else an executable
/// imports must ship with it.
fn is_windows_system_library(lower: &str) -> bool {
    const SYSTEM: &[&str] = &[
        "kernel32.dll",
        "kernelbase.dll",
        "ntdll.dll",
        "user32.dll",
        "gdi32.dll",
        "advapi32.dll",
        "shell32.dll",
        "ole32.dll",
        "oleaut32.dll",
        "comctl32.dll",
        "comdlg32.dll",
        "ws2_32.dll",
        "bcrypt.dll",
        "bcryptprimitives.dll",
        "crypt32.dll",
        "secur32.dll",
        "userenv.dll",
        "shlwapi.dll",
        "imm32.dll",
        "winmm.dll",
        "dwmapi.dll",
        "uxtheme.dll",
        "dxgi.dll",
        "d3d11.dll",
        "d3d12.dll",
        "d3dcompiler_47.dll",
        "dwrite.dll",
        "d2d1.dll",
        "opengl32.dll",
        "version.dll",
        "setupapi.dll",
        "hid.dll",
        "propsys.dll",
        "msvcrt.dll",
        "ucrtbase.dll",
        "ntoskrnl.exe",
        "iphlpapi.dll",
        "powrprof.dll",
        "dbghelp.dll",
        "rpcrt4.dll",
        "winhttp.dll",
        "wininet.dll",
        "mfplat.dll",
        "mf.dll",
        "mfreadwrite.dll",
        "avrt.dll",
        "mmdevapi.dll",
        "windowscodecs.dll",
        "oleacc.dll",
        "uiautomationcore.dll",
        "coremessaging.dll",
        "combase.dll",
        "cfgmgr32.dll",
        "api-ms-win-core-synch-l1-2-0.dll",
    ];
    lower.starts_with("api-ms-win-") || lower.starts_with("ext-ms-") || SYSTEM.contains(&lower)
}

/// `LC_RPATH` entries and `LC_LOAD_DYLIB`-family install names of a
/// Mach-O executable.
fn macho_load_paths(file: &object::File<'_>) -> (Vec<String>, Vec<String>) {
    use object::read::macho::LoadCommandVariant;
    macro_rules! collect {
        ($macho:expr) => {{
            let mut rpaths = Vec::new();
            let mut dylibs = Vec::new();
            let endian = $macho.endian();
            if let Ok(mut commands) = $macho.macho_load_commands() {
                while let Ok(Some(command)) = commands.next() {
                    match command.variant() {
                        Ok(LoadCommandVariant::Rpath(rpath)) => {
                            if let Ok(path) = command.string(endian, rpath.path) {
                                rpaths.push(String::from_utf8_lossy(path).into_owned());
                            }
                        }
                        Ok(LoadCommandVariant::Dylib(dylib))
                            if command.cmd() != object::macho::LC_ID_DYLIB =>
                        {
                            if let Ok(name) = command.string(endian, dylib.dylib.name) {
                                dylibs.push(String::from_utf8_lossy(name).into_owned());
                            }
                        }
                        _ => {}
                    }
                }
            }
            (rpaths, dylibs)
        }};
    }
    match file {
        object::File::MachO64(macho) => collect!(macho),
        object::File::MachO32(macho) => collect!(macho),
        _ => (Vec::new(), Vec::new()),
    }
}

fn elf_dynamic(file: &object::File<'_>) -> (Vec<String>, Vec<String>) {
    use object::elf;
    let mut needed = Vec::new();
    let mut runpath = Vec::new();
    macro_rules! collect {
        ($elf:expr) => {
            if let Ok(table) = $elf.elf_dynamic_table() {
                for entry in table.iter() {
                    let Ok(text) = entry.string(table.strings()) else {
                        continue;
                    };
                    let text = String::from_utf8_lossy(text).into_owned();
                    match entry.tag {
                        elf::DT_NEEDED => needed.push(text),
                        elf::DT_RUNPATH | elf::DT_RPATH => runpath.push(text),
                        _ => {}
                    }
                }
            }
        };
    }
    match file {
        object::File::Elf64(elf) => collect!(elf),
        object::File::Elf32(elf) => collect!(elf),
        _ => {}
    }
    (needed, runpath)
}

fn check_packs(
    report: &mut ValidationReport,
    app_root: &Path,
    layout: &PackageLayout,
    manifest: &PackageManifest,
    trust: &TrustPolicy,
) {
    let resources = PackageLayout::join(app_root, &layout.runtime_resources);
    for pack in &manifest.resource_packs {
        let name = format!("pack.{}", pack.name);
        let Some(keys) = pack_keys(pack) else {
            let var = pack
                .key_name
                .as_deref()
                .map(content_key_env)
                .unwrap_or_default();
            report.push(
                name,
                Status::NotExecuted(format!("encrypted; set {var} to verify its contents")),
            );
            continue;
        };
        // `PackageManifest::read` rejected any pack whose pin does not parse.
        let expected = pack.expected();
        let status = match PackReader::open(
            &resources.join(&pack.file),
            &pack.name,
            &keys,
            trust,
            expected.as_ref(),
        ) {
            Err(error) => Status::Fail(error.to_string()),
            Ok(reader) => match reader.keys() {
                Err(error) => Status::Fail(error.to_string()),
                Ok(entries) => {
                    let failed: Vec<String> = entries
                        .iter()
                        .filter_map(|key| {
                            reader
                                .read(key, u64::MAX, &mut ReadStats::default())
                                .err()
                                .map(|error| format!("{key}: {error}"))
                        })
                        .collect();
                    if failed.is_empty() {
                        Status::Pass
                    } else {
                        Status::Fail(failed.join("; "))
                    }
                }
            },
        };
        report.push(name, status);
    }
}

fn check_distribution(
    report: &mut ValidationReport,
    app_root: &Path,
    manifest: &PackageManifest,
) -> Result<(), String> {
    let mut problems = Vec::new();
    if manifest.distribution.self_update {
        problems.push("manifest declares a self-update helper".to_owned());
    }
    for path in walk_files(app_root)? {
        let lower = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if lower.contains("updater") || lower.starts_with("nana-update") {
            problems.push(format!(
                "{} looks like an updater",
                relative_path(app_root, &path)?
            ));
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        if contains(&bytes, UPDATER_MARKER) {
            problems.push(format!(
                "{} carries the updater marker",
                relative_path(app_root, &path)?
            ));
        }
    }
    report.push(
        "distribution.no-updater",
        if problems.is_empty() {
            Status::Pass
        } else if manifest.distribution.backend == DistributionBackend::Steam {
            Status::Fail(problems.join("; "))
        } else {
            Status::Warn(problems.join("; "))
        },
    );
    Ok(())
}

/// What one self-check launch produced.
struct SelfCheck {
    success: bool,
    status: String,
    /// The `{"nana_package_validate":…}` line, when the application printed one.
    report: Option<String>,
    stderr: String,
}

impl SelfCheck {
    /// The application ran its self-check and every check passed.
    fn passed(&self) -> bool {
        self.success
            && self
                .report
                .as_deref()
                .is_some_and(|r| r.contains("\"ok\":true"))
    }

    /// The application ran its self-check and reported a failure (as opposed
    /// to crashing, hanging, or never reaching the check).
    fn reported_failure(&self) -> bool {
        !self.success
            && self
                .report
                .as_deref()
                .is_some_and(|r| r.contains("\"ok\":false"))
    }

    fn describe(&self) -> String {
        match &self.report {
            Some(report) => format!("{} {report}", self.status),
            None if self.success => "the executable exited without a self-check report; does it \
                                     start through nana_ui::NanaApplication::builder(...).start() \
                                     with the `packaged-resources` feature?"
                .into(),
            None => format!("{}: {}", self.status, self.stderr.trim()),
        }
    }
}

/// Launch the executable from an unrelated working directory in self-check
/// mode. Output is drained on threads so a chatty child cannot block on a
/// full pipe; a child that does not finish within 60 s is killed.
fn self_check(
    exe: &Path,
    env: &[(String, String)],
    drop_env: &[String],
) -> Result<SelfCheck, String> {
    let cwd = std::env::temp_dir().join(format!("nana-validate-cwd-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    command
        .current_dir(&cwd)
        .env(SELF_CHECK_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        if !drop_env.contains(key) {
            command.env(key, value);
        }
    }
    for key in drop_env {
        command.env_remove(key);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot launch {}: {error}", exe.display()))?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("self-check did not finish within 60 s".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    let _ = std::fs::remove_dir_all(&cwd);
    let stdout = String::from_utf8_lossy(&stdout);
    Ok(SelfCheck {
        success: status.success(),
        status: status.to_string(),
        report: stdout
            .lines()
            .find(|line| line.starts_with(SELF_CHECK_PREFIX))
            .map(str::to_owned),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// Copy the package, break one thing, and expect the self-check to fail.
fn tamper_suite(
    report: &mut ValidationReport,
    app_root: &Path,
    located: &Located,
    manifest: &PackageManifest,
    env: &[(String, String)],
    drop_env: &[String],
) -> Result<(), String> {
    let layout = &located.layout;
    let resource = |file: &str| format!("{}/{file}", layout.runtime_resources);
    // The control run comes first: if the untouched copy fails its
    // self-check, a failing tampered copy proves nothing.
    let mut cases = vec![Case::new("control", Tamper::None)];
    // The opening `{`: invalid JSON whether or not the manifest is signed.
    cases.push(Case::new(
        "manifest-byte",
        Tamper::Flip(
            format!(
                "{}/{}",
                layout.runtime_manifest,
                nana_package::MANIFEST_FILE
            ),
            Offset::At(0),
        ),
    ));
    if let Some(pack) = manifest.resource_packs.first() {
        let pack = resource(&pack.file);
        cases.push(Case::new(
            "pack-header",
            Tamper::Flip(pack.clone(), Offset::At(format::ENTRY_COUNT_OFFSET as u64)),
        ));
        cases.push(Case::new(
            "pack-toc",
            Tamper::Flip(pack.clone(), Offset::Last),
        ));
        match first_record(app_root, layout, manifest) {
            // A byte inside an entry's stored record (never padding).
            Some((file, offset)) => cases.push(Case::new(
                "pack-data",
                Tamper::Flip(resource(&file), Offset::At(offset)),
            )),
            None => report.push(
                "tamper.pack-data",
                Status::NotExecuted(
                    "no pack could be opened to locate a record (encrypted packs need their \
                     NANA_CONTENT_KEY_<NAME>)"
                        .into(),
                ),
            ),
        }
        cases.push(Case::new("pack-missing", Tamper::Remove(pack)));
    }
    if !drop_env.is_empty() {
        // Same package, the application's key source removed.
        cases.push(Case {
            name: "key-missing",
            tamper: Tamper::None,
            drop_env,
        });
    }
    for case in cases {
        let control = case.name == "control";
        let copy = std::env::temp_dir().join(format!(
            "nana-validate-tamper-{}-{}/{}",
            case.name,
            std::process::id(),
            layout.root
        ));
        let _ = std::fs::remove_dir_all(copy.parent().expect("has parent"));
        copy_dir(app_root, &copy)?;
        case.tamper.apply(&copy)?;
        let exe = PackageLayout::join(&copy, &layout.executable);
        let status = match self_check(&exe, env, case.drop_env) {
            Ok(check) if control && check.passed() => Status::Pass,
            Ok(check) if control => Status::Fail(format!(
                "the untouched copy fails its self-check, so tampering proves nothing: {}",
                check.describe()
            )),
            Ok(check) if check.reported_failure() => Status::Pass,
            Ok(check) if check.passed() => {
                Status::Fail("the tampered package still passed its self-check".into())
            }
            Ok(check) => Status::Fail(format!(
                "expected the self-check to report the damage, got: {}",
                check.describe()
            )),
            Err(error) => Status::Fail(error),
        };
        let failed_control = control && !matches!(status, Status::Pass);
        report.push(format!("tamper.{}", case.name), status);
        let _ = std::fs::remove_dir_all(copy.parent().expect("has parent"));
        if failed_control {
            break;
        }
    }
    Ok(())
}

/// One tamper-suite run: damage the copy, optionally hide env variables.
struct Case<'a> {
    name: &'static str,
    tamper: Tamper,
    drop_env: &'a [String],
}

impl Case<'_> {
    fn new(name: &'static str, tamper: Tamper) -> Self {
        Self {
            name,
            tamper,
            drop_env: &[],
        }
    }
}

enum Tamper {
    None,
    /// Flip one bit of the file at this package-relative path.
    Flip(String, Offset),
    Remove(String),
}

enum Offset {
    At(u64),
    Last,
}

impl Tamper {
    fn apply(&self, root: &Path) -> Result<(), String> {
        match self {
            Self::None => Ok(()),
            Self::Remove(relative) => {
                std::fs::remove_file(PackageLayout::join(root, relative)).map_err(|e| e.to_string())
            }
            Self::Flip(relative, offset) => {
                let path = PackageLayout::join(root, relative);
                let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                let at = match offset {
                    Offset::At(at) => *at as usize,
                    Offset::Last => bytes.len().saturating_sub(1),
                };
                let byte = bytes
                    .get_mut(at)
                    .ok_or_else(|| format!("{relative} is too short to tamper with"))?;
                *byte ^= 0x01;
                // Replace the file instead of writing through it: a symlink
                // in the copy must not carry the damage back to the original.
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                std::fs::write(&path, bytes).map_err(|e| e.to_string())
            }
        }
    }
}

/// A pack file and the offset of its first stored record, from the first
/// pack that opens with the keys in the environment.
fn first_record(
    app_root: &Path,
    layout: &PackageLayout,
    manifest: &PackageManifest,
) -> Option<(String, u64)> {
    let resources = PackageLayout::join(app_root, &layout.runtime_resources);
    manifest.resource_packs.iter().find_map(|pack| {
        let keys = pack_keys(pack)?;
        let reader = PackReader::open(
            &resources.join(&pack.file),
            &pack.name,
            &keys,
            &TrustPolicy::AllowUnsigned,
            None,
        )
        .ok()?;
        reader.keys().ok()?.iter().find_map(|key| {
            let (entry, blocks) = reader.stored_entry(key).ok()?;
            (!blocks.is_empty()).then(|| (pack.file.clone(), entry.extent_offset))
        })
    })
}

/// Keys to open `pack` with: none for a plain pack, the content key from
/// `NANA_CONTENT_KEY_<NAME>` for an encrypted one (`None` when unset).
fn pack_keys(pack: &nana_package::manifest::ManifestPack) -> Option<StaticKeys> {
    match &pack.key_name {
        None => Some(StaticKeys::new()),
        Some(key_name) => std::env::var(content_key_env(key_name))
            .ok()
            .and_then(|text| ContentKey::from_hex(&text))
            .map(|key| {
                StaticKeys::new().with(KeyId::from_name(key_name), pack.key_generation, key)
            }),
    }
}

/// Read a child pipe to the end on its own thread, so a chatty child
/// cannot block on a full pipe while the caller waits.
fn drain<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

/// Copy a package tree for the tamper suite. Symlinks are recreated as
/// symlinks (a `.framework` links `Versions/Current`), not followed.
fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for item in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let target = to.join(item.file_name());
        let kind = item.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            let link = std::fs::read_link(item.path()).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link, &target).map_err(|e| e.to_string())?;
            #[cfg(windows)]
            {
                let resolved = item
                    .path()
                    .parent()
                    .map_or(link.clone(), |dir| dir.join(&link));
                if resolved.is_dir() {
                    std::os::windows::fs::symlink_dir(&link, &target)
                } else {
                    std::os::windows::fs::symlink_file(&link, &target)
                }
                .map_err(|e| e.to_string())?;
            }
        } else if kind.is_dir() {
            copy_dir(&item.path(), &target)?;
        } else {
            std::fs::copy(item.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_crt_names() {
        for name in [
            "vcruntime140d.dll",
            "vcruntime140_1d.dll",
            "msvcp140d.dll",
            "msvcp140d_atomic_wait.dll",
            "ucrtbased.dll",
        ] {
            assert!(is_debug_crt(name), "{name}");
        }
        for name in [
            "vcruntime140.dll",
            "msvcp140.dll",
            "msvcp140_atomic_wait.dll",
            "ucrtbase.dll",
        ] {
            assert!(!is_debug_crt(name), "{name}");
        }
    }
}
