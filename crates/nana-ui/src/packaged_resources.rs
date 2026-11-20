//! Packaged resources at startup (Issue #226): read the package manifest,
//! mount its `.nrpack` resource packs behind `nana://res/`, and answer the
//! packager's `NANA_PACKAGE_VALIDATE` self-check.
//!
//! Packs open lazily on their first lookup, so startup reads the manifest
//! and nothing else; a lookup routes to one pack by its path prefixes and
//! touches only that entry's blocks. Everything a pack returns has passed
//! its hash and authentication checks. Nothing here runs per frame: reads
//! happen on cache misses of the image, font and stylesheet loaders.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use nana_diagnostics::framework::{package as package_diag, resource as resource_diag};
use nana_diagnostics::{event, fault, metric};
use nana_package::manifest::PackageManifest;
use nana_package::{
    ExpectedPack, KeyProvider, ManifestError, NoKeys, PackError, PackReader, ReadStats,
    SignatureState, TrustPolicy,
};
use nana_ui_core::{PackagedReadError, PackagedResourceSource};
use nana_ui_platform::{ApplicationIdentity, ApplicationPaths, RuntimeLayout};

pub use nana_package::SELF_CHECK_ENV;

/// How to open the package's resource packs.
#[derive(Clone)]
pub struct ResourcePackOptions {
    keys: Arc<dyn KeyProvider>,
    trust: TrustPolicy,
    loose_root: Option<PathBuf>,
}

impl ResourcePackOptions {
    /// Unencrypted packs only, no publisher signature required.
    pub fn new() -> Self {
        Self {
            keys: Arc::new(NoKeys),
            trust: TrustPolicy::AllowUnsigned,
            loose_root: None,
        }
    }

    /// Where content keys come from. How the application obtains them
    /// (embedded, a license file, a platform API) is its own policy.
    pub fn keys(mut self, keys: impl KeyProvider + 'static) -> Self {
        self.keys = Arc::new(keys);
        self
    }

    /// Require every pack and the manifest to be signed by this publisher.
    pub fn trust(mut self, trust: TrustPolicy) -> Self {
        self.trust = trust;
        self
    }

    /// In the development layout (running from `target/`), serve logical
    /// paths from this directory, the packager's `resources.root`.
    pub fn loose_root(mut self, dir: impl Into<PathBuf>) -> Self {
        self.loose_root = Some(dir.into());
        self
    }
}

impl Default for ResourcePackOptions {
    fn default() -> Self {
        Self::new()
    }
}

static MANIFEST: OnceLock<PackageManifest> = OnceLock::new();
/// The mount the manifest describes, kept for the self-check and the Early
/// Splash read — also when another source already holds the `nana-ui-core`
/// hook (which otherwise holds a type-erased handle to this same mount).
static MOUNT: OnceLock<Arc<PackMount>> = OnceLock::new();
/// The installed development source, kept for the Early Splash logo read.
#[cfg_attr(not(feature = "hosted"), allow(dead_code))]
static LOOSE: OnceLock<Arc<LooseSource>> = OnceLock::new();

/// The package manifest the application started with, if it has one.
pub(crate) fn package_manifest() -> Option<&'static PackageManifest> {
    MANIFEST.get()
}

/// Where a pack's bytes are.
enum PackLocation {
    File(PathBuf),
    /// An APK asset path (Android), read through its byte range in the APK.
    Asset(String),
}

/// Where the package manifest and its packs were found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageRoot {
    /// `RuntimeManifest` / `RuntimeResources` of [`ApplicationPaths`].
    Paths,
    /// The APK's `assets/nana/` (Android).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    ApkAssets,
}

struct LazyPack {
    name: String,
    location: PackLocation,
    prefixes: Vec<String>,
    expected: Option<ExpectedPack>,
    reader: Mutex<Slot>,
    integrity_reported: AtomicBool,
    /// Retryable open failures (missing key, I/O) are reported once per pack.
    open_failure_reported: AtomicBool,
}

/// A pack's open state. Integrity failures are final; a missing key or an
/// I/O error is retried on the next lookup (a license or network key may
/// arrive later).
enum Slot {
    Closed,
    Open(Arc<PackReader>),
    Failed(PackError),
}

struct PackMount {
    packs: Vec<LazyPack>,
    keys: Arc<dyn KeyProvider>,
    trust: TrustPolicy,
}

impl PackMount {
    fn route(&self, path: &str) -> Option<&LazyPack> {
        self.packs
            .iter()
            .flat_map(|pack| pack.prefixes.iter().map(move |prefix| (prefix, pack)))
            .filter(|(prefix, _)| nana_package::manifest::prefix_claims(prefix, path))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, pack)| pack)
    }

    /// The pack's reader, opening it on first use. The key provider runs
    /// outside the lock, so it may take its time (it must not read the pack
    /// it is keying through `nana://res/`, which would recurse). Two threads
    /// racing on the first lookup may both open; one result is kept.
    fn reader(&self, pack: &LazyPack) -> Result<Arc<PackReader>, PackError> {
        match &*pack.reader.lock().unwrap_or_else(|e| e.into_inner()) {
            Slot::Open(reader) => return Ok(reader.clone()),
            Slot::Failed(error) => return Err(error.clone()),
            Slot::Closed => {}
        }
        let started = Instant::now();
        let opened = match &pack.location {
            PackLocation::File(path) => PackReader::open(
                path,
                &pack.name,
                self.keys.as_ref(),
                &self.trust,
                pack.expected.as_ref(),
            ),
            PackLocation::Asset(path) => asset_source(path).and_then(|source| {
                PackReader::open_source(
                    source,
                    &pack.name,
                    self.keys.as_ref(),
                    &self.trust,
                    pack.expected.as_ref(),
                )
            }),
        };
        let mut slot = pack.reader.lock().unwrap_or_else(|e| e.into_inner());
        if let Slot::Open(reader) = &*slot {
            return Ok(reader.clone());
        }
        match opened {
            Ok(reader) => {
                metric!(resource_diag::PACK_MOUNT_NS, started.elapsed());
                metric!(resource_diag::PACK_TOC_BYTES, reader.toc_len());
                event!(
                    resource_diag::PACK_MOUNTED,
                    entries = reader.entry_count() as u64,
                    toc_bytes = reader.toc_len(),
                    encrypted = reader.header().encrypted(),
                    signed = reader.header().signed(),
                );
                let reader = Arc::new(reader);
                *slot = Slot::Open(reader.clone());
                Ok(reader)
            }
            Err(error) => {
                if error.is_integrity_failure() {
                    metric!(resource_diag::INTEGRITY_FAILURES);
                }
                let retry = matches!(error, PackError::KeyUnavailable(_) | PackError::Io(_));
                // A final failure always reports (it happens once: the slot
                // keeps it); retryable ones report the first time only.
                if !retry || !pack.open_failure_reported.swap(true, Ordering::Relaxed) {
                    fault!(
                        resource_diag::PACK_OPEN_FAILED,
                        code = error.code();
                        "resource pack `{}` cannot be opened: {error}", pack.name
                    );
                }
                if !retry {
                    *slot = Slot::Failed(error.clone());
                }
                Err(error)
            }
        }
    }
}

impl PackMount {
    /// Read `path` from `pack`, opening it on first use, with the metrics and
    /// faults every packaged read records.
    fn read_entry(
        &self,
        pack: &LazyPack,
        path: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, PackError> {
        let reader = self.reader(pack)?;
        let started = Instant::now();
        let mut stats = ReadStats::default();
        let result = reader.read(path, max_bytes, &mut stats);
        metric!(resource_diag::READS);
        metric!(resource_diag::BYTES_READ, stats.bytes_read);
        match &result {
            Ok(_) => {
                metric!(resource_diag::ENTRY_READ_NS, started.elapsed());
                if stats.auth_ns > 0 {
                    metric!(resource_diag::ENTRY_AUTH_NS, stats.auth_ns);
                }
                if stats.decompress_ns > 0 {
                    metric!(resource_diag::ENTRY_DECOMPRESS_NS, stats.decompress_ns);
                }
            }
            Err(PackError::NotFound) => metric!(resource_diag::MISSES),
            Err(error) if error.is_integrity_failure() => {
                metric!(resource_diag::INTEGRITY_FAILURES);
                if !pack.integrity_reported.swap(true, Ordering::Relaxed) {
                    fault!(
                        resource_diag::INTEGRITY_FAILED,
                        code = error.code();
                        "resource pack `{}`: `{path}` failed verification: {error}", pack.name
                    );
                }
            }
            Err(_) => {}
        }
        result
    }
}

impl PackagedResourceSource for PackMount {
    fn read(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, PackagedReadError> {
        let Some(pack) = self.route(path) else {
            metric!(resource_diag::MISSES);
            return Err(PackagedReadError::NotFound);
        };
        self.read_entry(pack, path, max_bytes)
            .map_err(|error| map_error(&error))
    }
}

/// An uncompressed APK asset as a pack source.
fn asset_source(path: &str) -> Result<Box<dyn nana_package::PackSource>, PackError> {
    let range = nana_ui_platform::android_assets::open_asset_range(path).map_err(|error| {
        match error.kind() {
            std::io::ErrorKind::NotFound => PackError::Missing,
            _ => PackError::Io(error.to_string()),
        }
    })?;
    Ok(Box::new(nana_package::FileRange::new(
        range.file,
        range.start,
        range.len,
    )))
}

/// The manifest under `assets/nana/manifest/` of the APK, if it has one.
#[cfg(target_os = "android")]
fn read_asset_manifest(
    trust: &TrustPolicy,
) -> Result<(PackageManifest, SignatureState, u64), ManifestError> {
    use nana_ui_platform::android_assets::read_asset;
    let dir = nana_package::ANDROID_ASSET_MANIFEST_DIR;
    let io = |error: std::io::Error| ManifestError::Io(error.to_string());
    let bytes = read_asset(
        &format!("{dir}/{}", nana_package::MANIFEST_FILE),
        nana_package::manifest::MAX_MANIFEST_BYTES,
    )
    .map_err(|error| match error.kind() {
        std::io::ErrorKind::InvalidData => ManifestError::TooLarge,
        _ => io(error),
    })?
    .ok_or(ManifestError::Missing)?;
    let signature = read_asset(
        &format!("{dir}/{}", nana_package::MANIFEST_SIGNATURE_FILE),
        64,
    )
    .map_err(|error| match error.kind() {
        std::io::ErrorKind::InvalidData => ManifestError::SignatureInvalid,
        _ => io(error),
    })?;
    let (manifest, state) =
        PackageManifest::from_signed_bytes(&bytes, signature.as_deref(), trust)?;
    Ok((manifest, state, bytes.len() as u64))
}

/// Read the package manifest: from the APK's assets on Android when it
/// carries one, else from `RuntimeManifest`.
fn read_manifest(
    paths: &ApplicationPaths,
    trust: &TrustPolicy,
) -> (
    Result<(PackageManifest, SignatureState, u64), ManifestError>,
    PackageRoot,
) {
    #[cfg(target_os = "android")]
    match read_asset_manifest(trust) {
        Err(ManifestError::Missing) => {}
        other => return (other, PackageRoot::ApkAssets),
    }
    let read = PackageManifest::read(paths.runtime_manifest(), trust).map(|(manifest, state)| {
        let bytes = std::fs::metadata(paths.runtime_manifest().join(nana_package::MANIFEST_FILE))
            .map_or(0, |meta| meta.len());
        (manifest, state, bytes)
    });
    (read, PackageRoot::Paths)
}

/// Where a file the manifest lists (relative to the application root of the
/// packaged tree) is installed: under the resolved logical location its
/// path starts with, so a Linux FHS install that moved `resources` and
/// `manifest` to `/usr/share/<pkg>/` is checked where it actually is.
fn installed_path(
    paths: &ApplicationPaths,
    layout: &nana_package::manifest::ManifestLayout,
    relative: &str,
) -> PathBuf {
    let join = |base: &std::path::Path, rest: &str| {
        rest.split('/')
            .filter(|part| !part.is_empty())
            .fold(base.to_path_buf(), |dir, part| dir.join(part))
    };
    if relative == layout.executable {
        return paths.executable().to_path_buf();
    }
    let mut locations = [
        (&layout.runtime_manifest, paths.runtime_manifest()),
        (&layout.runtime_resources, paths.runtime_resources()),
        (&layout.runtime_bin, paths.runtime_bin()),
        (&layout.runtime_plugins, paths.runtime_plugins()),
        (&layout.runtime_tools, paths.runtime_tools()),
    ];
    // macOS nests the manifest inside Resources: the longest prefix wins.
    locations.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
    for (prefix, resolved) in locations {
        if let Some(rest) = relative.strip_prefix(prefix.as_str())
            && let Some(rest) = rest.strip_prefix('/')
        {
            return join(resolved, rest);
        }
    }
    join(paths.app_root(), relative)
}

fn map_error(error: &PackError) -> PackagedReadError {
    match error {
        PackError::NotFound | PackError::Missing => PackagedReadError::NotFound,
        PackError::TooLarge { .. } => PackagedReadError::TooLarge,
        PackError::KeyUnavailable(_) => PackagedReadError::KeyUnavailable,
        PackError::Io(_) => PackagedReadError::Io,
        _ => PackagedReadError::Integrity,
    }
}

/// Development: logical paths straight from the resource source directory.
struct LooseSource {
    root: PathBuf,
}

impl LooseSource {
    /// `path` is already a validated logical path: join its segments (no
    /// second URL decoding), then require it to stay inside the root.
    fn locate(&self, path: &str) -> Option<PathBuf> {
        let file = path
            .split('/')
            .fold(self.root.clone(), |dir, part| dir.join(part));
        nana_ui_core::canonicalize_within_jail(&file, &self.root)
    }
}

/// Why a loose read failed; `TooLarge` carries the length seen.
#[cfg_attr(not(feature = "hosted"), allow(dead_code))]
enum LooseReadError {
    NotFound,
    TooLarge(u64),
    Io(String),
}

impl LooseSource {
    /// At most `max_bytes`, however the file changes while it is read.
    fn read_bounded(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, LooseReadError> {
        use std::io::Read;
        let file = self.locate(path).ok_or(LooseReadError::NotFound)?;
        let file = std::fs::File::open(&file).map_err(|_| LooseReadError::NotFound)?;
        let len = file.metadata().map_err(|_| LooseReadError::NotFound)?.len();
        if len > max_bytes {
            return Err(LooseReadError::TooLarge(len));
        }
        let mut bytes = Vec::new();
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| LooseReadError::Io(error.to_string()))?;
        if bytes.len() as u64 > max_bytes {
            return Err(LooseReadError::TooLarge(bytes.len() as u64));
        }
        Ok(bytes)
    }
}

impl PackagedResourceSource for LooseSource {
    fn read(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, PackagedReadError> {
        self.read_bounded(path, max_bytes)
            .map_err(|error| match error {
                LooseReadError::NotFound => PackagedReadError::NotFound,
                LooseReadError::TooLarge(_) => PackagedReadError::TooLarge,
                LooseReadError::Io(_) => PackagedReadError::Io,
            })
    }
}

/// Read the Early Splash logo `url` names (Issue #225): one entry of the
/// `early-splash` pack the manifest routes it to, through the same mount,
/// cached reader and diagnostics as every `nana://res/` read. A path in a pack
/// of another class is refused before anything is opened; the reader caps the
/// pack file after its header and the entry before its data. Early-splash
/// packs are never encrypted, so no key provider runs. In the development
/// layout without a manifest, the one file under the loose root, where there
/// is no class to check.
#[cfg(feature = "hosted")]
pub(crate) fn read_splash_logo(url: &str) -> Result<Vec<u8>, crate::startup::SplashFailure> {
    use crate::startup::{
        MAX_LOGO_ENCODED_BYTES, SplashFailure, SplashLogoError, SplashPackageError as E,
    };
    use nana_package::EarlySplashError;

    let package = |error| SplashFailure::Package(error);
    let too_large = |bytes: u64| {
        SplashFailure::Logo(SplashLogoError::TooLarge {
            bytes: usize::try_from(bytes).unwrap_or(usize::MAX),
        })
    };
    let path = nana_ui_core::packaged_logical_path(url, None).ok_or(package(E::InvalidUrl))?;
    let max = MAX_LOGO_ENCODED_BYTES as u64;
    let (Some(manifest), Some(mount)) = (MANIFEST.get(), MOUNT.get()) else {
        let loose = LOOSE.get().ok_or(package(E::NotMounted))?;
        return loose.read_bounded(&path, max).map_err(|error| match error {
            LooseReadError::NotFound => package(E::NotFound),
            LooseReadError::TooLarge(len) => too_large(len),
            LooseReadError::Io(reason) => package(E::Io(reason)),
        });
    };
    let pack = manifest.early_splash_pack(&path).map_err(|error| {
        package(match error {
            EarlySplashError::InvalidPath => E::InvalidUrl,
            EarlySplashError::NotFound => E::NotFound,
            EarlySplashError::WrongClass { pack, class } => E::NotEarlySplash {
                pack,
                class: class.as_str(),
            },
        })
    })?;
    let lazy = mount
        .packs
        .iter()
        .find(|lazy| lazy.name == pack.name)
        .ok_or(package(E::NotFound))?;
    mount
        .read_entry(lazy, &path, max)
        .map_err(|error| match error {
            PackError::NotFound => package(E::NotFound),
            PackError::TooLarge { len } => too_large(len),
            error => package(E::Pack {
                pack: lazy.name.clone(),
                code: error.code(),
                reason: error.to_string(),
            }),
        })
}

/// Which `nana://res/` source startup installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Packs,
    Loose,
}

/// What startup found, for the self-check.
struct Mounted {
    manifest: Result<(&'static PackageManifest, SignatureState), ManifestError>,
    root: PackageRoot,
    identity_matches: Option<bool>,
    source: Option<Source>,
}

fn identity_matches(identity: &ApplicationIdentity, manifest: &PackageManifest) -> bool {
    manifest.application.matches(
        &identity.id,
        &identity.name,
        &identity.version,
        identity.vendor.as_deref(),
    )
}

fn mount(
    identity: &ApplicationIdentity,
    paths: Option<&ApplicationPaths>,
    options: &ResourcePackOptions,
) -> Mounted {
    let Some(paths) = paths else {
        return Mounted {
            manifest: Err(ManifestError::Io("application paths are unresolved".into())),
            root: PackageRoot::Paths,
            identity_matches: None,
            source: None,
        };
    };
    let started = Instant::now();
    let (read, root) = read_manifest(paths, &options.trust);
    let mut bytes = 0;
    let manifest = read.map(|(manifest, state, len)| {
        bytes = len;
        // Kept for the life of the process; the first builder's wins.
        (MANIFEST.get_or_init(|| manifest), state)
    });
    metric!(package_diag::MANIFEST_READ_NS, started.elapsed());
    let mut mounted = Mounted {
        manifest,
        root,
        identity_matches: None,
        source: None,
    };
    match &mounted.manifest {
        Ok((manifest, state)) => {
            event!(
                package_diag::MANIFEST_LOADED,
                bytes = bytes,
                packs = manifest.resource_packs.len() as u64,
                signed = *state == SignatureState::Verified,
            );
            let matches = identity_matches(identity, manifest);
            mounted.identity_matches = Some(matches);
            if !matches {
                fault!(
                    package_diag::IDENTITY_MISMATCH;
                    "the package manifest describes {} {}, the binary is {} {}",
                    manifest.application.id, manifest.application.version,
                    identity.id, identity.version
                );
            }
            let resources = paths.runtime_resources();
            let packs = manifest
                .resource_packs
                .iter()
                .map(|pack| LazyPack {
                    name: pack.name.clone(),
                    location: match root {
                        PackageRoot::Paths => PackLocation::File(resources.join(&pack.file)),
                        PackageRoot::ApkAssets => PackLocation::Asset(format!(
                            "{}/{}",
                            nana_package::ANDROID_ASSET_RESOURCES_DIR,
                            pack.file
                        )),
                    },
                    prefixes: pack.prefixes.clone(),
                    expected: pack.expected(),
                    reader: Mutex::new(Slot::Closed),
                    integrity_reported: AtomicBool::new(false),
                    open_failure_reported: AtomicBool::new(false),
                })
                .collect();
            let mount = Arc::new(PackMount {
                packs,
                keys: options.keys.clone(),
                trust: options.trust.clone(),
            });
            let _ = MOUNT.set(mount.clone());
            let installed = nana_ui_core::install_packaged_source(mount);
            mounted.source = installed.then_some(Source::Packs);
        }
        Err(ManifestError::Missing) if paths.layout() == RuntimeLayout::Development => {
            if let Some(root) = &options.loose_root {
                let loose = Arc::new(LooseSource { root: root.clone() });
                let installed = nana_ui_core::install_packaged_source(loose.clone());
                if installed {
                    let _ = LOOSE.set(loose);
                }
                mounted.source = installed.then_some(Source::Loose);
            }
        }
        Err(ManifestError::Missing) => event!(package_diag::MANIFEST_MISSING),
        Err(error) => {
            fault!(
                package_diag::MANIFEST_INVALID,
                code = error.code();
                "{error}"
            );
        }
    }
    mounted
}

/// Called from `NanaApplicationBuilder::start`. Returns the process exit
/// code when startup was a self-check (the caller shuts diagnostics down,
/// so faults recorded here reach the log, and exits).
///
/// `splash_logo` is the `nana://res/` URL of the application's packaged
/// Early Splash logo, if it declared one; the self-check reads it exactly as
/// the host would.
pub(crate) fn start(
    identity: &ApplicationIdentity,
    paths: Option<&ApplicationPaths>,
    options: Option<&ResourcePackOptions>,
    splash_logo: Option<&'static str>,
) -> Option<i32> {
    let self_check = std::env::var_os(SELF_CHECK_ENV).is_some_and(|v| v == "1");
    let default = ResourcePackOptions::new();
    let mounted = options.map(|options| mount(identity, paths, options));
    self_check.then(|| {
        let mounted = mounted.unwrap_or_else(|| mount(identity, paths, &default));
        run_self_check(paths, &mounted, splash_logo)
    })
}

#[derive(serde::Serialize)]
struct CheckLine {
    name: &'static str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

/// Print `{"nana_package_validate":1,...}` and return the exit code: 0 when
/// every check passed, 3 otherwise. Never prints key material.
fn run_self_check(
    paths: Option<&ApplicationPaths>,
    mounted: &Mounted,
    splash_logo: Option<&'static str>,
) -> i32 {
    let mut checks = Vec::new();
    let mut push = |name, ok, detail: Option<String>| checks.push(CheckLine { name, ok, detail });

    match paths {
        Some(paths) => {
            let layout = paths.layout();
            push(
                "paths.layout",
                layout != RuntimeLayout::Development,
                Some(layout.as_str().into()),
            );
        }
        None => push("paths.layout", false, Some("unresolved".into())),
    }
    match &mounted.manifest {
        Ok((manifest, state)) => {
            push("manifest.read", true, Some(format!("{state:?}")));
            // Delivery channels drop empty directories; what must exist is
            // every file the manifest lists, where this install keeps it. An
            // APK's files are the package installer's to check.
            if let (Some(paths), PackageRoot::Paths) = (paths, mounted.root) {
                let missing: Vec<&str> = manifest
                    .runtime_files
                    .iter()
                    .filter(|file| !installed_path(paths, &manifest.layout, &file.path).is_file())
                    .map(|file| file.path.as_str())
                    .collect();
                push(
                    "files.present",
                    missing.is_empty(),
                    (!missing.is_empty()).then(|| missing.join(", ")),
                );
            }
            push(
                "identity.matches-manifest",
                mounted.identity_matches == Some(true),
                None,
            );
            push(
                "resources.source",
                mounted.source == Some(Source::Packs),
                mounted.source.map(|source| format!("{source:?}")),
            );
            for pack in &manifest.resource_packs {
                let ok = pack_readable(pack);
                push(
                    "pack.entries",
                    ok.is_ok(),
                    Some(match ok {
                        Ok(()) => pack.name.clone(),
                        Err(error) => format!("{}: {error}", pack.name),
                    }),
                );
            }
        }
        Err(error) => push(
            "manifest.read",
            false,
            Some(format!("{error} (code {})", error.code())),
        ),
    }
    #[cfg(feature = "hosted")]
    if let Some(url) = splash_logo {
        let (ok, detail) = splash_logo_check(url);
        push("startup.early-splash-logo", ok, Some(detail));
    }
    #[cfg(not(feature = "hosted"))]
    let _ = splash_logo;
    let ok = checks.iter().all(|c| c.ok);
    // Fixed key order: the validator finds the line by its prefix.
    let checks = serde_json::to_string(&checks).expect("checks serialize");
    println!(
        "{}1,\"ok\":{ok},\"checks\":{checks}}}",
        nana_package::SELF_CHECK_PREFIX
    );
    if ok { 0 } else { 3 }
}

/// The packaged Early Splash logo, read and header-checked as the host
/// would before showing it. The native splash itself needs a window.
#[cfg(feature = "hosted")]
fn splash_logo_check(url: &str) -> (bool, String) {
    use crate::startup::{SplashFailure, validate_logo};
    match read_splash_logo(url).and_then(|png| validate_logo(&png).map_err(SplashFailure::Logo)) {
        Ok(info) => (true, format!("{url}: {}x{}", info.width, info.height)),
        Err(failure) => (false, format!("{url}: {failure}")),
    }
}

/// Open `pack` with the application's keys and read every entry: the first
/// through the installed `nana://res/` hook, exactly as a stylesheet or
/// image would (routing included), the rest directly. Self-check only;
/// normal startup never walks a pack.
fn pack_readable(pack: &nana_package::manifest::ManifestPack) -> Result<(), String> {
    let mount = MOUNT.get().ok_or("no pack mount")?;
    let lazy = mount
        .packs
        .iter()
        .find(|lazy| lazy.name == pack.name)
        .ok_or("pack is not mounted")?;
    let reader = mount
        .reader(lazy)
        .map_err(|error| format!("{error} (code {})", error.code()))?;
    let keys = reader.keys().map_err(|error| error.to_string())?;
    let first = keys.first().ok_or("pack is empty")?;
    if nana_ui_core::read_packaged(&nana_ui_core::packaged_url(first), None, u64::MAX).is_none() {
        return Err(
            match reader.read(first, u64::MAX, &mut ReadStats::default()) {
                Err(error) => format!("{first}: {error} (code {})", error.code()),
                Ok(_) => format!("{first}: the nana://res/ hook routes elsewhere"),
            },
        );
    }
    for key in &keys[1..] {
        reader
            .read(key, u64::MAX, &mut ReadStats::default())
            .map_err(|error| format!("{key}: {error} (code {})", error.code()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lazy(name: &str, prefixes: &[&str]) -> LazyPack {
        LazyPack {
            name: name.into(),
            location: PackLocation::File(PathBuf::from(format!("/nonexistent/{name}.nrpack"))),
            prefixes: prefixes.iter().map(|p| (*p).to_owned()).collect(),
            expected: None,
            reader: Mutex::new(Slot::Closed),
            integrity_reported: AtomicBool::new(false),
            open_failure_reported: AtomicBool::new(false),
        }
    }

    fn mount(packs: Vec<LazyPack>) -> PackMount {
        PackMount {
            packs,
            keys: Arc::new(NoKeys),
            trust: TrustPolicy::AllowUnsigned,
        }
    }

    #[test]
    fn listed_files_are_checked_where_the_install_keeps_them() {
        use nana_ui_platform::{PathEnvironment, PathPlatform};
        let resolve = |platform, exe: &str| {
            let env = PathEnvironment {
                platform,
                executable: PathBuf::from(exe),
                vars: [("HOME", "/home/u"), ("XDG_DATA_DIRS", "/nonexistent-nana")]
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v.into()))
                    .collect(),
                android_files_dir: None,
                android_cache_dir: None,
                portable_marker: false,
            };
            let identity = ApplicationIdentity::new("dev.nana.fixture", "Fixture", "1.0.0");
            ApplicationPaths::resolve_with(&identity, &env).unwrap()
        };
        let tree = nana_package::manifest::ManifestLayout {
            executable: "Fixture".into(),
            runtime_bin: "runtime/bin".into(),
            runtime_resources: "runtime/resources".into(),
            runtime_plugins: "runtime/plugins".into(),
            runtime_tools: "runtime/tools".into(),
            runtime_manifest: "runtime/manifest".into(),
            root_exceptions: vec![],
        };
        let fhs = resolve(PathPlatform::Linux, "/usr/lib/fixture/Fixture");
        for (listed, installed) in [
            ("Fixture", "/usr/lib/fixture/Fixture"),
            (
                "runtime/resources/ui.nrpack",
                "/usr/share/fixture/resources/ui.nrpack",
            ),
            (
                "runtime/manifest/portable",
                "/usr/share/fixture/manifest/portable",
            ),
            (
                "runtime/bin/libfoo.so",
                "/usr/lib/fixture/runtime/bin/libfoo.so",
            ),
            // Only a whole path segment matches a location.
            (
                "runtime/resourcesx/a",
                "/usr/lib/fixture/runtime/resourcesx/a",
            ),
        ] {
            assert_eq!(
                installed_path(&fhs, &tree, listed),
                PathBuf::from(installed),
                "{listed}"
            );
        }
        let bundle = nana_package::manifest::ManifestLayout {
            executable: "Contents/MacOS/Fixture".into(),
            runtime_bin: "Contents/Frameworks".into(),
            runtime_resources: "Contents/Resources".into(),
            runtime_plugins: "Contents/PlugIns".into(),
            runtime_tools: "Contents/Helpers".into(),
            runtime_manifest: "Contents/Resources/manifest".into(),
            root_exceptions: vec![],
        };
        let app = resolve(
            PathPlatform::MacOs,
            "/Applications/Fixture.app/Contents/MacOS/Fixture",
        );
        assert_eq!(
            installed_path(&app, &bundle, "Contents/Resources/manifest/x"),
            PathBuf::from("/Applications/Fixture.app/Contents/Resources/manifest/x")
        );
        assert_eq!(
            installed_path(&app, &bundle, "Contents/Info.plist"),
            PathBuf::from("/Applications/Fixture.app/Contents/Info.plist")
        );
    }

    #[test]
    fn lookups_route_by_longest_prefix() {
        let mount = mount(vec![
            lazy("ui", &["ui/"]),
            lazy("fonts", &["ui/fonts/"]),
            lazy("readme", &["readme.txt"]),
        ]);
        let route = |path| mount.route(path).map(|pack| pack.name.as_str());
        assert_eq!(route("ui/app.css"), Some("ui"));
        assert_eq!(route("ui/fonts/a.ttf"), Some("fonts"));
        assert_eq!(route("readme.txt"), Some("readme"));
        assert_eq!(route("readme.txt.bak"), None);
        assert_eq!(route("uix/a.css"), None);
    }

    #[test]
    fn a_missing_pack_is_not_found_and_stays_closed_for_good() {
        let mount = mount(vec![lazy("ui", &["ui/"])]);
        assert_eq!(
            mount.read("ui/a.css", 1024),
            Err(PackagedReadError::NotFound)
        );
        assert!(matches!(
            &*mount.packs[0].reader.lock().unwrap(),
            Slot::Failed(PackError::Missing)
        ));
        assert_eq!(
            mount.read("other/a.css", 1024),
            Err(PackagedReadError::NotFound)
        );
    }

    #[test]
    fn loose_source_reads_logical_paths_inside_its_root_only() {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("nana-loose-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ui")).unwrap();
        std::fs::write(root.join("ui/a%41.css"), b"x").unwrap();
        std::fs::write(root.join("ui/big.bin"), [0u8; 16]).unwrap();
        let source = LooseSource { root: root.clone() };
        // No second percent-decoding: the logical path names the file as is.
        assert_eq!(source.read("ui/a%41.css", 16), Ok(b"x".to_vec()));
        assert_eq!(
            source.read("ui/big.bin", 8),
            Err(PackagedReadError::TooLarge)
        );
        assert_eq!(
            source.read("ui/missing", 8),
            Err(PackagedReadError::NotFound)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_and_missing_vendor_are_the_same_identity() {
        let mut manifest = PackageManifest::from_json(
            br#"{"schema":"nana.package-manifest","schema_version":1,
            "application":{"id":"a.b","name":"A","version":"1","vendor":"","build_id":null},
            "target":{"triple":"t","platform":"linux"},"nana":{"version":"0"},
            "build":{"profile":"dist","debug_assertions_suspected":false},
            "static_app_plan":{"status":"unavailable","reason":"r"},
            "distribution":{"backend":"portable","self_update":false},
            "layout":{"executable":"A","runtime_bin":"runtime/bin","runtime_resources":"runtime/resources",
              "runtime_plugins":"runtime/plugins","runtime_tools":"runtime/tools",
              "runtime_manifest":"runtime/manifest","root_exceptions":[]},
            "resource_packs":[],"plugins":[],"runtime_files":[],
            "signing":{"publisher":{"state":"not-configured"},"publisher_key":null,"platform":[]}}"#,
        )
        .unwrap();
        let identity = ApplicationIdentity::new("a.b", "A", "1");
        assert!(identity_matches(&identity, &manifest));
        manifest.application.vendor = Some("V".into());
        assert!(!identity_matches(&identity, &manifest));
        assert!(identity_matches(&identity.vendor("V"), &manifest));
    }
}
