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
use std::sync::OnceLock;

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
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "windows") {
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

const VARS: [&str; 7] = [
    "HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
];

/// Where the portable marker lives for an executable outside a bundle.
pub fn portable_marker_path(executable: &Path) -> Option<PathBuf> {
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
        let platform =
            PathPlatform::current().ok_or(PathsError::UnsupportedPlatform(std::env::consts::OS))?;
        let executable =
            std::env::current_exe().map_err(|e| PathsError::NoExecutable(e.to_string()))?;
        // Resolve symlinks so a launcher link does not move the app root.
        let executable = std::fs::canonicalize(&executable)
            .map(strip_verbatim)
            .unwrap_or(executable);
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

/// Windows `canonicalize` returns verbatim paths (`\\?\C:\…`), in which
/// `/` is not a separator and many tools refuse to open anything. Turn the
/// plain drive and UNC forms back into ordinary paths.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    // Only when the plain form means the same file: short enough for the
    // legacy APIs, and no component ending in `.` or ` ` (which the plain
    // form would silently trim).
    match text.strip_prefix(r"\\?\") {
        Some(rest)
            if rest.as_bytes().get(1) == Some(&b':')
                && rest.len() < 260
                && !rest
                    .split('\\')
                    .any(|part| part.ends_with('.') || part.ends_with(' ')) =>
        {
            PathBuf::from(rest)
        }
        _ => path,
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

/// An executable under Cargo's `target/<profile>/` (or its `deps` /
/// `examples` subdirectories).
fn is_cargo_target(executable: &Path) -> bool {
    executable
        .ancestors()
        .skip(1)
        .take(4)
        .any(|dir| dir.file_name().is_some_and(|name| name == "target"))
}

/// Stricter than `app_data_dir`'s historical rule: the id also names log
/// files and directories on every platform, so characters Windows reserves
/// in file names are rejected too.
fn valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && !app_id.starts_with(['/', '\\'])
        && !app_id.contains(['\0', ':', '<', '>', '"', '|', '?', '*'])
        && app_id
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// The resolved locations for one application on one machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationPaths {
    identity: ApplicationIdentity,
    layout: RuntimeLayout,
    locations: BTreeMap<ApplicationLocation, PathBuf>,
}

static CURRENT: OnceLock<ApplicationPaths> = OnceLock::new();

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
            (None, PathPlatform::Android) => {}
            (None, _) => {
                let runtime = exe_dir.join("runtime");
                map.insert(L::AppRoot, exe_dir.clone());
                map.insert(L::RuntimeBin, runtime.join("bin"));
                map.insert(L::RuntimeResources, runtime.join("resources"));
                map.insert(L::RuntimePlugins, runtime.join("plugins"));
                map.insert(L::RuntimeTools, runtime.join("tools"));
                map.insert(L::RuntimeManifest, runtime.join("manifest"));
            }
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
                        env.var(var)
                            .filter(|p| p.is_absolute())
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
                    let runtime = files.join("runtime");
                    map.insert(L::AppRoot, files.clone());
                    map.insert(L::RuntimeBin, runtime.join("bin"));
                    map.insert(L::RuntimeResources, runtime.join("resources"));
                    map.insert(L::RuntimePlugins, runtime.join("plugins"));
                    map.insert(L::RuntimeTools, runtime.join("tools"));
                    map.insert(L::RuntimeManifest, runtime.join("manifest"));
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

    /// Publish these paths as the process's application paths and return
    /// the published value. The first call wins: later calls return the
    /// paths already published (compare with `==` to detect that).
    pub fn install_current(self) -> &'static Self {
        CURRENT.get_or_init(|| self)
    }

    /// The paths published by [`Self::install_current`], if any.
    pub fn current() -> Option<&'static Self> {
        CURRENT.get()
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
    fn verbatim_windows_paths_become_ordinary_paths() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\Games\App.exe")),
            PathBuf::from(r"C:\Games\App.exe")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share\App.exe")),
            PathBuf::from(r"\\server\share\App.exe")
        );
        let long = format!(r"\\?\C:\{}\App.exe", "d".repeat(300));
        assert_eq!(strip_verbatim(PathBuf::from(&long)), PathBuf::from(&long));
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\odd.\App.exe")),
            PathBuf::from(r"\\?\C:\odd.\App.exe")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from("/opt/app/app")),
            PathBuf::from("/opt/app/app")
        );
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
