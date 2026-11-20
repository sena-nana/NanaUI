//! Application identity and logical locations (Issue #226 §2, used by the
//! diagnostics runtime of Issue #227).
//!
//! Applications ask for a logical location — runtime resources, writable
//! data, logs — instead of joining `./runtime/...` or platform directories
//! themselves. [`ApplicationPaths::resolve`] maps each location onto the
//! running platform and layout:
//!
//! | location | Windows | macOS | Linux | portable |
//! | --- | --- | --- | --- | --- |
//! | runtime_* | `<exe dir>/runtime/{bin,resources,plugins,tools,manifest}` | inside `.app`: `Contents/{Frameworks,Resources,PlugIns,Helpers,Resources/manifest}` | as Windows | as Windows |
//! | data | `%APPDATA%\{id}` | `~/Library/Application Support/{id}` | `$XDG_DATA_HOME/{id}` | `<root>/data` |
//! | config | `%APPDATA%\{id}\config` | `…/Application Support/{id}/config` | `$XDG_CONFIG_HOME/{id}` | `<root>/data/config` |
//! | cache | `%LOCALAPPDATA%\{id}\Cache` | `~/Library/Caches/{id}` | `$XDG_CACHE_HOME/{id}` | `<root>/data/cache` |
//! | logs | `%LOCALAPPDATA%\{id}\Logs` | `~/Library/Logs/{id}` | `$XDG_STATE_HOME/{id}/logs` | `<root>/data/logs` |
//! | crash | `%LOCALAPPDATA%\{id}\Crash` | `~/Library/Logs/{id}/Crash` | `$XDG_STATE_HOME/{id}/crash` | `<root>/data/crash` |
//!
//! A Linux executable installed in FHS form — `<prefix>/lib/<pkg>/<exe>` or
//! `<prefix>/libexec/<pkg>/<exe>`, where the prefix is `/usr`, `/usr/local`,
//! an AppImage's `$APPDIR/usr` or Flatpak's `/app` — keeps `runtime/{bin,
//! plugins,tools}` beside it, and reads its architecture-independent
//! `resources` and `manifest` from `<data>/<pkg>/`: the first of
//! `<prefix>/share` and the `$XDG_DATA_DIRS` entries that holds
//! `<pkg>/manifest`, else `<prefix>/share`.
//!
//! A build is portable when `runtime/manifest/portable` exists next to the
//! executable (not supported inside a macOS bundle, which is read-only once
//! signed). Android keeps everything under the app's private files directory;
//! APK assets are not filesystem paths, so `runtime_*` there are writable
//! extraction targets under `data`.
//!
//! Nothing here creates directories. Writable ones are created on demand by
//! [`ApplicationPaths::ensure`].

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Who the application is. `id` names its per-user directories, so keep it
/// stable across releases (reverse-DNS is conventional).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationIdentity {
    pub id: String,
    pub name: String,
    pub version: String,
    pub vendor: Option<String>,
    pub build_id: Option<String>,
}

impl ApplicationIdentity {
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            vendor: None,
            build_id: None,
        }
    }

    pub fn vendor(mut self, vendor: impl Into<String>) -> Self {
        self.vendor = Some(vendor.into());
        self
    }

    pub fn build_id(mut self, build_id: impl Into<String>) -> Self {
        self.build_id = Some(build_id.into());
        self
    }
}

/// Declare the application identity and embed it in the binary.
///
/// Expands to an [`ApplicationIdentity`] expression and, as a side effect,
/// keeps a `NANA-IDENTITY-V1` marker in the executable's read-only data. The
/// packager reads that marker to refuse packaging a binary whose identity
/// disagrees with `nana-package.toml`, and the final-artifact validator
/// compares it with the package manifest (Issue #226). Arguments must be
/// string literals or literal-producing macros such as
/// `env!("CARGO_PKG_VERSION")`.
///
/// ```
/// let identity = nana_ui_platform::application_identity!(
///     id: "dev.nana.example",
///     name: "Nana Example",
///     version: env!("CARGO_PKG_VERSION"),
///     vendor: "Nana",
/// );
/// assert_eq!(identity.id, "dev.nana.example");
/// ```
#[macro_export]
macro_rules! application_identity {
    (id: $id:expr, name: $name:expr, version: $version:expr, vendor: $vendor:expr $(,)?) => {
        $crate::__application_identity!($id, $name, $version, $vendor).vendor($vendor)
    };
    (id: $id:expr, name: $name:expr, version: $version:expr $(,)?) => {
        $crate::__application_identity!($id, $name, $version, "")
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __application_identity {
    ($id:expr, $name:expr, $version:expr, $vendor:expr) => {{
        const TEXT: &str = concat!(
            "NANA-IDENTITY-V1\0",
            $id,
            "\0",
            $name,
            "\0",
            $version,
            "\0",
            $vendor,
            "\0END\0"
        );
        #[used]
        static MARKER: [u8; TEXT.len()] = $crate::__identity_marker_bytes::<{ TEXT.len() }>(TEXT);
        // Keep the marker alive through fat LTO and `--gc-sections`.
        ::core::hint::black_box(&MARKER);
        $crate::ApplicationIdentity::new($id, $name, $version)
    }};
}

/// Copy the marker text into an array, rejecting at compile time what the
/// packager's parser (`nana_package::identity`) would not accept: a NUL or
/// control character inside a field, a field over 256 bytes, an empty id or
/// version.
#[doc(hidden)]
pub const fn __identity_marker_bytes<const N: usize>(text: &str) -> [u8; N] {
    const MAGIC_LEN: usize = "NANA-IDENTITY-V1\0".len();
    const MAX_FIELD: usize = 256;
    let bytes = text.as_bytes();
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        out[i] = bytes[i];
        i += 1;
    }
    // Fields: id, name, version, vendor; then "END\0".
    let mut field = 0;
    let mut field_len = 0;
    let mut at = MAGIC_LEN;
    while field < 4 {
        let b = bytes[at];
        if b == 0 {
            if field_len == 0 && (field == 0 || field == 2) {
                panic!("application_identity!: id and version must not be empty");
            }
            field += 1;
            field_len = 0;
        } else {
            // C0, DEL, and C1 (U+0080..=U+009F, UTF-8 `C2 80..=9F`): what
            // `char::is_control` rejects in the parser.
            let c1 = b == 0xc2 && at + 1 < N && bytes[at + 1] >= 0x80 && bytes[at + 1] <= 0x9f;
            if b < 0x20 || b == 0x7f || c1 {
                panic!("application_identity!: fields must not contain control characters or NUL");
            }
            field_len += 1;
            if field_len > MAX_FIELD {
                panic!("application_identity!: a field is longer than 256 bytes");
            }
        }
        at += 1;
    }
    if N - at != 4 {
        panic!("application_identity!: fields must not contain NUL");
    }
    out
}

/// How the application is laid out on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLayout {
    /// Installed by an installer, Steam, or a platform package; per-user
    /// data lives in the platform's standard directories.
    Installed,
    /// Self-contained folder: writable data lives under `<root>/data`.
    Portable,
    /// Running from a Cargo `target/` directory. Runtime directories may not
    /// exist; writable ones are the platform's standard directories.
    Development,
}

impl RuntimeLayout {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Portable => "portable",
            Self::Development => "development",
        }
    }
}

/// A logical location. See the module docs for each platform's mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ApplicationLocation {
    Executable,
    AppRoot,
    RuntimeBin,
    RuntimeResources,
    RuntimePlugins,
    RuntimeTools,
    RuntimeManifest,
    Data,
    Config,
    Cache,
    Logs,
    Crash,
}

impl ApplicationLocation {
    pub const ALL: [Self; 12] = [
        Self::Executable,
        Self::AppRoot,
        Self::RuntimeBin,
        Self::RuntimeResources,
        Self::RuntimePlugins,
        Self::RuntimeTools,
        Self::RuntimeManifest,
        Self::Data,
        Self::Config,
        Self::Cache,
        Self::Logs,
        Self::Crash,
    ];

    /// Per-user locations the application may write to.
    pub const fn is_writable(self) -> bool {
        matches!(
            self,
            Self::Data | Self::Config | Self::Cache | Self::Logs | Self::Crash
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathsError {
    /// Empty, absolute, containing `.` / `..` / empty segments, or any of
    /// NUL `:` `<` `>` `"` `|` `?` `*`.
    InvalidAppId(String),
    /// The running executable's path is unknown.
    NoExecutable(String),
    /// A variable the platform mapping needs (`HOME`, `APPDATA`...) is unset.
    MissingEnvironment(&'static str),
    UnsupportedPlatform(&'static str),
    NotWritable(ApplicationLocation),
}

impl fmt::Display for PathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAppId(id) => write!(f, "invalid application id `{id}`"),
            Self::NoExecutable(e) => write!(f, "cannot locate the executable: {e}"),
            Self::MissingEnvironment(var) => write!(f, "environment variable {var} is not set"),
            Self::UnsupportedPlatform(os) => write!(f, "no path mapping for platform `{os}`"),
            Self::NotWritable(location) => write!(f, "{location:?} is not a writable location"),
        }
    }
}

impl std::error::Error for PathsError {}

/// Target platform of a path mapping. Separate from `cfg!(target_os)` so the
/// mapping of every platform can be tested anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPlatform {
    Windows,
    MacOs,
    Linux,
    Android,
}

impl PathPlatform {
    /// `None` on OHOS: it reports `target_os = "linux"`, but no XDG layout
    /// applies inside its sandbox (`docs/reference/ohos.md`).
    pub fn current() -> Option<Self> {
        if cfg!(target_env = "ohos") {
            None
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else if cfg!(target_os = "android") {
            Some(Self::Android)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// Everything the mapping reads from the outside world.
#[derive(Debug, Clone)]
pub struct PathEnvironment {
    pub platform: PathPlatform,
    pub executable: PathBuf,
    /// `HOME`, `APPDATA`, `LOCALAPPDATA`, `XDG_*`.
    pub vars: BTreeMap<String, OsString>,
    /// Android `Context.getFilesDir()` / `getCacheDir()`.
    pub android_files_dir: Option<PathBuf>,
    pub android_cache_dir: Option<PathBuf>,
    /// Whether a portable marker file exists at the given path.
    pub portable_marker: bool,
}

const VARS: [&str; 8] = [
    "HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_DATA_DIRS",
];

/// Where the portable marker lives for an executable outside a bundle.
fn portable_marker_path(executable: &Path) -> Option<PathBuf> {
    Some(
        executable
            .parent()?
            .join("runtime")
            .join("manifest")
            .join("portable"),
    )
}

impl PathEnvironment {
    /// Read the running process's environment.
    pub fn current() -> Result<Self, PathsError> {
        let platform = PathPlatform::current().ok_or(PathsError::UnsupportedPlatform(
            if cfg!(target_env = "ohos") {
                "ohos"
            } else {
                std::env::consts::OS
            },
        ))?;
        let executable =
            std::env::current_exe().map_err(|e| PathsError::NoExecutable(e.to_string()))?;
        // Resolve symlinks so a launcher link does not move the app root.
        let executable = crate::path::canonicalize_path(&executable).unwrap_or(executable);
        let vars = VARS
            .iter()
            .filter_map(|name| {
                std::env::var_os(name)
                    .filter(|v| !v.is_empty())
                    .map(|v| ((*name).to_owned(), v))
            })
            .collect();
        let portable_marker = platform != PathPlatform::Android
            && macos_bundle_root(&executable).is_none()
            && portable_marker_path(&executable).is_some_and(|p| p.is_file());
        #[cfg(target_os = "android")]
        let (android_files_dir, android_cache_dir) = (
            crate::persist::android_dir("getFilesDir"),
            crate::persist::android_dir("getCacheDir"),
        );
        #[cfg(not(target_os = "android"))]
        let (android_files_dir, android_cache_dir) = (None, None);
        Ok(Self {
            platform,
            executable,
            vars,
            android_files_dir,
            android_cache_dir,
            portable_marker,
        })
    }

    fn var(&self, name: &'static str) -> Option<PathBuf> {
        self.vars.get(name).map(PathBuf::from)
    }

    fn require(&self, name: &'static str) -> Result<PathBuf, PathsError> {
        self.var(name).ok_or(PathsError::MissingEnvironment(name))
    }
}

/// `…/Name.app/Contents/MacOS/exe` → `…/Name.app`.
fn macos_bundle_root(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

/// `<prefix>/lib/<pkg>` (or `libexec`) → `(prefix, pkg)`, for the prefixes
/// a Linux package installs into: `/usr`, `/usr/local`, an AppImage's
/// `$APPDIR/usr`, Flatpak's `/app`.
fn linux_fhs(exe_dir: &Path) -> Option<(PathBuf, String)> {
    let pkg = exe_dir.file_name()?.to_str()?.to_owned();
    let lib = exe_dir.parent()?;
    if !matches!(lib.file_name()?.to_str()?, "lib" | "libexec") {
        return None;
    }
    let prefix = lib.parent()?;
    let usr_local = prefix.file_name()? == "local"
        && prefix.parent().and_then(Path::file_name) == Some("usr".as_ref());
    let accepted = prefix.file_name()? == "usr" || usr_local || prefix == Path::new("/app");
    accepted.then(|| (prefix.to_path_buf(), pkg))
}

/// Where an FHS installation keeps `<pkg>/{resources,manifest}`.
fn linux_data_dir(env: &PathEnvironment, prefix: &Path, pkg: &str) -> PathBuf {
    let own = prefix.join("share");
    let dirs = env
        .vars
        .get("XDG_DATA_DIRS")
        .and_then(|value| value.to_str())
        .unwrap_or("/usr/local/share:/usr/share");
    std::iter::once(own.clone())
        .chain(
            dirs.split(':')
                .filter(|dir| dir.starts_with('/'))
                .map(PathBuf::from),
        )
        .find(|dir| dir.join(pkg).join("manifest").is_dir())
        .unwrap_or(own)
}

/// An executable under Cargo's `target/<profile>/` (or its `deps` /
/// `examples` subdirectories).
fn is_cargo_target(executable: &Path) -> bool {
    executable
        .ancestors()
        .skip(1)
        .take(4)
        .any(|dir| dir.file_name().is_some_and(|name| name == "target"))
}

/// `app_data_dir`'s rule, plus the characters Windows reserves in file
/// names: the id also names log files and directories on every platform.
fn valid_app_id(app_id: &str) -> bool {
    crate::persist::valid_app_id(app_id) && !app_id.contains([':', '<', '>', '"', '|', '?', '*'])
}

/// The resolved locations for one application on one machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationPaths {
    identity: ApplicationIdentity,
    layout: RuntimeLayout,
    locations: BTreeMap<ApplicationLocation, PathBuf>,
}

impl ApplicationPaths {
    /// Resolve for the running process.
    pub fn resolve(identity: &ApplicationIdentity) -> Result<Self, PathsError> {
        Self::resolve_with(identity, &PathEnvironment::current()?)
    }

    /// Resolve against an explicit environment (tests, tools that inspect
    /// another platform's layout).
    pub fn resolve_with(
        identity: &ApplicationIdentity,
        env: &PathEnvironment,
    ) -> Result<Self, PathsError> {
        if !valid_app_id(&identity.id) {
            return Err(PathsError::InvalidAppId(identity.id.clone()));
        }
        let id = identity.id.as_str();
        let exe = env.executable.clone();
        let exe_dir = exe
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| PathsError::NoExecutable("executable has no parent".into()))?;

        use ApplicationLocation as L;
        let mut map = BTreeMap::new();
        map.insert(L::Executable, exe.clone());

        let bundle = (env.platform == PathPlatform::MacOs)
            .then(|| macos_bundle_root(&exe))
            .flatten();
        // Android has no application folder to be portable in.
        let layout =
            if env.portable_marker && bundle.is_none() && env.platform != PathPlatform::Android {
                RuntimeLayout::Portable
            } else if bundle.is_none() && is_cargo_target(&exe) {
                RuntimeLayout::Development
            } else {
                RuntimeLayout::Installed
            };

        // `<root>/runtime/{bin,resources,plugins,tools,manifest}`.
        let runtime_tree = |map: &mut BTreeMap<_, _>, root: &Path| {
            let runtime = root.join("runtime");
            map.insert(L::AppRoot, root.to_path_buf());
            map.insert(L::RuntimeBin, runtime.join("bin"));
            map.insert(L::RuntimeResources, runtime.join("resources"));
            map.insert(L::RuntimePlugins, runtime.join("plugins"));
            map.insert(L::RuntimeTools, runtime.join("tools"));
            map.insert(L::RuntimeManifest, runtime.join("manifest"));
        };

        // Read-only runtime locations.
        match (&bundle, env.platform) {
            (Some(bundle), _) => {
                let contents = bundle.join("Contents");
                map.insert(L::AppRoot, bundle.clone());
                map.insert(L::RuntimeBin, contents.join("Frameworks"));
                map.insert(L::RuntimeResources, contents.join("Resources"));
                map.insert(L::RuntimePlugins, contents.join("PlugIns"));
                map.insert(L::RuntimeTools, contents.join("Helpers"));
                map.insert(
                    L::RuntimeManifest,
                    contents.join("Resources").join("manifest"),
                );
            }
            // Android's tree lives under the files directory, below.
            (None, PathPlatform::Android) => {}
            (None, PathPlatform::Linux) if layout == RuntimeLayout::Installed => {
                runtime_tree(&mut map, &exe_dir);
                if let Some((prefix, pkg)) = linux_fhs(&exe_dir) {
                    let data = linux_data_dir(env, &prefix, &pkg).join(&pkg);
                    map.insert(L::RuntimeResources, data.join("resources"));
                    map.insert(L::RuntimeManifest, data.join("manifest"));
                }
            }
            (None, _) => runtime_tree(&mut map, &exe_dir),
        }

        // Writable per-user locations.
        if layout == RuntimeLayout::Portable {
            let data = exe_dir.join("data");
            map.insert(L::Config, data.join("config"));
            map.insert(L::Cache, data.join("cache"));
            map.insert(L::Logs, data.join("logs"));
            map.insert(L::Crash, data.join("crash"));
            map.insert(L::Data, data);
        } else {
            match env.platform {
                PathPlatform::Windows => {
                    let roaming = env.require("APPDATA")?.join(id);
                    let local = env.require("LOCALAPPDATA")?.join(id);
                    map.insert(L::Config, roaming.join("config"));
                    map.insert(L::Data, roaming);
                    map.insert(L::Cache, local.join("Cache"));
                    map.insert(L::Logs, local.join("Logs"));
                    map.insert(L::Crash, local.join("Crash"));
                }
                PathPlatform::MacOs => {
                    let library = env.require("HOME")?.join("Library");
                    let support = library.join("Application Support").join(id);
                    let logs = library.join("Logs").join(id);
                    map.insert(L::Config, support.join("config"));
                    map.insert(L::Data, support);
                    map.insert(L::Cache, library.join("Caches").join(id));
                    map.insert(L::Crash, logs.join("Crash"));
                    map.insert(L::Logs, logs);
                }
                PathPlatform::Linux => {
                    let home = env.var("HOME");
                    let xdg = |var: &'static str, fallback: &str| {
                        // XDG ignores a relative value. Absolute means POSIX
                        // absolute, not whatever the host this runs on calls
                        // absolute: `Path::is_absolute` wants a drive letter
                        // on Windows and would drop every well-formed value
                        // when this platform is resolved from there.
                        env.var(var)
                            .filter(|p| p.starts_with("/"))
                            .or_else(|| home.as_ref().map(|h| h.join(fallback)))
                            .ok_or(PathsError::MissingEnvironment("HOME"))
                    };
                    let state = xdg("XDG_STATE_HOME", ".local/state")?.join(id);
                    map.insert(L::Data, xdg("XDG_DATA_HOME", ".local/share")?.join(id));
                    map.insert(L::Config, xdg("XDG_CONFIG_HOME", ".config")?.join(id));
                    map.insert(L::Cache, xdg("XDG_CACHE_HOME", ".cache")?.join(id));
                    map.insert(L::Logs, state.join("logs"));
                    map.insert(L::Crash, state.join("crash"));
                }
                PathPlatform::Android => {
                    let files = env
                        .android_files_dir
                        .clone()
                        .ok_or(PathsError::MissingEnvironment("Context.getFilesDir"))?;
                    let cache = env
                        .android_cache_dir
                        .clone()
                        .unwrap_or_else(|| files.join("cache"));
                    runtime_tree(&mut map, &files);
                    map.insert(L::Config, files.join("config"));
                    map.insert(L::Logs, files.join("logs"));
                    map.insert(L::Crash, files.join("crash"));
                    map.insert(L::Cache, cache);
                    map.insert(L::Data, files);
                }
            }
        }

        Ok(Self {
            identity: identity.clone(),
            layout,
            locations: map,
        })
    }

    pub fn identity(&self) -> &ApplicationIdentity {
        &self.identity
    }

    pub fn layout(&self) -> RuntimeLayout {
        self.layout
    }

    pub fn get(&self, location: ApplicationLocation) -> &Path {
        // Every variant is inserted by `resolve_with`.
        &self.locations[&location]
    }

    /// Create a writable location (and its parents) if missing.
    pub fn ensure(&self, location: ApplicationLocation) -> Result<&Path, io::Error> {
        if !location.is_writable() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                PathsError::NotWritable(location),
            ));
        }
        let path = self.get(location);
        std::fs::create_dir_all(path)?;
        Ok(path)
    }

    pub fn executable(&self) -> &Path {
        self.get(ApplicationLocation::Executable)
    }
    pub fn app_root(&self) -> &Path {
        self.get(ApplicationLocation::AppRoot)
    }
    pub fn runtime_bin(&self) -> &Path {
        self.get(ApplicationLocation::RuntimeBin)
    }
    pub fn runtime_resources(&self) -> &Path {
        self.get(ApplicationLocation::RuntimeResources)
    }
    pub fn runtime_plugins(&self) -> &Path {
        self.get(ApplicationLocation::RuntimePlugins)
    }
    pub fn runtime_tools(&self) -> &Path {
        self.get(ApplicationLocation::RuntimeTools)
    }
    pub fn runtime_manifest(&self) -> &Path {
        self.get(ApplicationLocation::RuntimeManifest)
    }
    pub fn data(&self) -> &Path {
        self.get(ApplicationLocation::Data)
    }
    pub fn config(&self) -> &Path {
        self.get(ApplicationLocation::Config)
    }
    pub fn cache(&self) -> &Path {
        self.get(ApplicationLocation::Cache)
    }
    pub fn logs(&self) -> &Path {
        self.get(ApplicationLocation::Logs)
    }
    pub fn crash(&self) -> &Path {
        self.get(ApplicationLocation::Crash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(platform: PathPlatform, exe: &str, vars: &[(&str, &str)]) -> PathEnvironment {
        PathEnvironment {
            platform,
            executable: PathBuf::from(exe),
            vars: vars
                .iter()
                .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
                .collect(),
            android_files_dir: None,
            android_cache_dir: None,
            portable_marker: false,
        }
    }

    #[test]
    fn identity_macro_embeds_a_marker() {
        let identity = crate::application_identity!(
            id: "dev.nana.marker",
            name: "Marker",
            version: "2.0.0",
            vendor: "Nana",
        );
        assert_eq!(
            identity,
            ApplicationIdentity::new("dev.nana.marker", "Marker", "2.0.0").vendor("Nana")
        );
        let bare = crate::application_identity!(id: "a.b", name: "A", version: "1");
        assert_eq!(bare.vendor, None);
        // The marker is in this test binary's data.
        let binary = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let marker = b"NANA-IDENTITY-V1\x00dev.nana.marker\x00Marker\x002.0.0\x00Nana\x00END\x00";
        assert!(binary.windows(marker.len()).any(|w| w == marker));
    }

    fn id() -> ApplicationIdentity {
        ApplicationIdentity::new("dev.nana.live", "NanaLive", "1.0.0")
    }

    #[test]
    fn windows_installed_layout_uses_roaming_and_local_appdata() {
        let e = env(
            PathPlatform::Windows,
            "C:/Games/NanaLive/NanaLive.exe",
            &[("APPDATA", "C:/U/R"), ("LOCALAPPDATA", "C:/U/L")],
        );
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(p.layout(), RuntimeLayout::Installed);
        assert_eq!(p.app_root(), Path::new("C:/Games/NanaLive"));
        assert_eq!(
            p.runtime_tools(),
            Path::new("C:/Games/NanaLive/runtime/tools")
        );
        assert_eq!(p.data(), Path::new("C:/U/R/dev.nana.live"));
        assert_eq!(p.config(), Path::new("C:/U/R/dev.nana.live/config"));
        assert_eq!(p.cache(), Path::new("C:/U/L/dev.nana.live/Cache"));
        assert_eq!(p.logs(), Path::new("C:/U/L/dev.nana.live/Logs"));
        assert_eq!(p.crash(), Path::new("C:/U/L/dev.nana.live/Crash"));
    }

    #[test]
    fn portable_marker_keeps_writable_data_beside_the_executable() {
        let mut e = env(PathPlatform::Windows, "D:/NanaLive/NanaLive.exe", &[]);
        e.portable_marker = true;
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(p.layout(), RuntimeLayout::Portable);
        assert_eq!(p.data(), Path::new("D:/NanaLive/data"));
        assert_eq!(p.logs(), Path::new("D:/NanaLive/data/logs"));
        assert_eq!(p.crash(), Path::new("D:/NanaLive/data/crash"));
        // Portable needs no environment variables at all.
    }

    #[test]
    fn macos_bundle_maps_runtime_into_contents() {
        let e = env(
            PathPlatform::MacOs,
            "/Applications/NanaLive.app/Contents/MacOS/NanaLive",
            &[("HOME", "/Users/u")],
        );
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(p.layout(), RuntimeLayout::Installed);
        assert_eq!(p.app_root(), Path::new("/Applications/NanaLive.app"));
        assert_eq!(
            p.runtime_bin(),
            Path::new("/Applications/NanaLive.app/Contents/Frameworks")
        );
        assert_eq!(
            p.runtime_manifest(),
            Path::new("/Applications/NanaLive.app/Contents/Resources/manifest")
        );
        assert_eq!(
            p.data(),
            Path::new("/Users/u/Library/Application Support/dev.nana.live")
        );
        assert_eq!(
            p.cache(),
            Path::new("/Users/u/Library/Caches/dev.nana.live")
        );
        assert_eq!(p.logs(), Path::new("/Users/u/Library/Logs/dev.nana.live"));
        assert_eq!(
            p.crash(),
            Path::new("/Users/u/Library/Logs/dev.nana.live/Crash")
        );
    }

    #[test]
    fn macos_bundle_ignores_the_portable_marker() {
        let mut e = env(
            PathPlatform::MacOs,
            "/Applications/NanaLive.app/Contents/MacOS/NanaLive",
            &[("HOME", "/Users/u")],
        );
        e.portable_marker = true;
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(p.layout(), RuntimeLayout::Installed);
    }

    #[test]
    fn linux_prefers_absolute_xdg_and_falls_back_to_home() {
        let e = env(
            PathPlatform::Linux,
            "/opt/nanalive/nanalive",
            &[
                ("HOME", "/home/u"),
                ("XDG_DATA_HOME", "/xdg/data"),
                ("XDG_CACHE_HOME", "relative/ignored"),
            ],
        );
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(p.data(), Path::new("/xdg/data/dev.nana.live"));
        assert_eq!(p.cache(), Path::new("/home/u/.cache/dev.nana.live"));
        assert_eq!(p.config(), Path::new("/home/u/.config/dev.nana.live"));
        assert_eq!(
            p.logs(),
            Path::new("/home/u/.local/state/dev.nana.live/logs")
        );
        assert_eq!(
            p.runtime_plugins(),
            Path::new("/opt/nanalive/runtime/plugins")
        );
    }

    #[test]
    fn linux_fhs_installs_read_resources_from_share() {
        for (exe, share) in [
            ("/usr/lib/nana-live/NanaLive", "/usr/share/nana-live"),
            ("/usr/libexec/nana-live/NanaLive", "/usr/share/nana-live"),
            (
                "/usr/local/lib/nana-live/NanaLive",
                "/usr/local/share/nana-live",
            ),
            // Flatpak and a mounted AppImage.
            ("/app/lib/nana-live/NanaLive", "/app/share/nana-live"),
            (
                "/tmp/.mount_NanaLiAbC/usr/lib/nana-live/NanaLive",
                "/tmp/.mount_NanaLiAbC/usr/share/nana-live",
            ),
        ] {
            let e = env(
                PathPlatform::Linux,
                exe,
                &[("HOME", "/home/u"), ("XDG_DATA_DIRS", "/nonexistent-nana")],
            );
            let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
            let exe_dir = Path::new(exe).parent().unwrap();
            assert_eq!(p.layout(), RuntimeLayout::Installed, "{exe}");
            assert_eq!(p.app_root(), exe_dir, "{exe}");
            assert_eq!(p.runtime_bin(), exe_dir.join("runtime/bin"), "{exe}");
            assert_eq!(
                p.runtime_resources(),
                Path::new(share).join("resources"),
                "{exe}"
            );
            assert_eq!(
                p.runtime_manifest(),
                Path::new(share).join("manifest"),
                "{exe}"
            );
            assert_eq!(p.data(), Path::new("/home/u/.local/share/dev.nana.live"));
        }
        // Not an FHS prefix: the portable-style tree beside the executable.
        for exe in ["/opt/lib/nana-live/NanaLive", "/home/u/apps/lib/x/x"] {
            let e = env(PathPlatform::Linux, exe, &[("HOME", "/home/u")]);
            let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
            let exe_dir = Path::new(exe).parent().unwrap();
            assert_eq!(p.runtime_resources(), exe_dir.join("runtime/resources"));
        }
        // FHS shapes are a Linux rule only.
        let e = env(
            PathPlatform::Windows,
            "/usr/lib/nana-live/NanaLive.exe",
            &[("APPDATA", "C:/R"), ("LOCALAPPDATA", "C:/L")],
        );
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(
            p.runtime_resources(),
            Path::new("/usr/lib/nana-live/runtime/resources")
        );
    }

    #[test]
    fn linux_fhs_data_comes_from_the_xdg_data_dir_that_holds_the_manifest() {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("nana-fhs-{}-{seq}", std::process::id()));
        let elsewhere = base.join("vendor-share");
        std::fs::create_dir_all(elsewhere.join("nana-live/manifest")).unwrap();
        let empty = base.join("empty-share");
        std::fs::create_dir_all(&empty).unwrap();
        let dirs = format!(
            "relative/ignored:{}:{}",
            empty.display(),
            elsewhere.display()
        );
        let e = env(
            PathPlatform::Linux,
            "/usr/lib/nana-live/NanaLive",
            &[("HOME", "/home/u"), ("XDG_DATA_DIRS", &dirs)],
        );
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(
            p.runtime_manifest(),
            elsewhere.join("nana-live").join("manifest")
        );
        assert_eq!(
            p.runtime_resources(),
            elsewhere.join("nana-live").join("resources")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cargo_target_executables_are_development_builds() {
        let e = env(
            PathPlatform::Linux,
            "/src/NanaUI/target/debug/examples/application-counter",
            &[("HOME", "/home/u")],
        );
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(p.layout(), RuntimeLayout::Development);
    }

    #[test]
    fn android_keeps_everything_in_private_storage() {
        let mut e = env(PathPlatform::Android, "/system/bin/app_process64", &[]);
        e.android_files_dir = Some("/data/user/0/dev.nana.live/files".into());
        e.android_cache_dir = Some("/data/user/0/dev.nana.live/cache".into());
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(p.data(), Path::new("/data/user/0/dev.nana.live/files"));
        assert_eq!(p.cache(), Path::new("/data/user/0/dev.nana.live/cache"));
        assert_eq!(p.logs(), Path::new("/data/user/0/dev.nana.live/files/logs"));
        assert!(p.runtime_resources().starts_with(p.data()));
    }

    #[test]
    fn android_ignores_a_portable_marker() {
        let mut e = env(PathPlatform::Android, "/system/bin/app_process64", &[]);
        e.android_files_dir = Some("/data/user/0/a/files".into());
        e.portable_marker = true;
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        assert_eq!(p.layout(), RuntimeLayout::Installed);
        for location in ApplicationLocation::ALL {
            let _ = p.get(location);
        }
    }

    #[test]
    fn missing_environment_and_bad_ids_are_errors() {
        let e = env(PathPlatform::Windows, "C:/a/b.exe", &[("APPDATA", "C:/R")]);
        assert_eq!(
            ApplicationPaths::resolve_with(&id(), &e),
            Err(PathsError::MissingEnvironment("LOCALAPPDATA"))
        );
        for bad in ["", "../x", "/abs", "a//b", "c:evil", "a|b", "what?"] {
            let identity = ApplicationIdentity::new(bad, "x", "1");
            assert!(matches!(
                ApplicationPaths::resolve_with(&identity, &e),
                Err(PathsError::InvalidAppId(_))
            ));
        }
    }

    #[test]
    fn read_only_locations_refuse_ensure() {
        let e = env(PathPlatform::Linux, "/opt/a/a", &[("HOME", "/home/u")]);
        let p = ApplicationPaths::resolve_with(&id(), &e).unwrap();
        let err = p.ensure(ApplicationLocation::RuntimeBin).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        for location in ApplicationLocation::ALL {
            let _ = p.get(location);
        }
    }
}
