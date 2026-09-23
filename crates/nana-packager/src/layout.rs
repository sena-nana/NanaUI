//! Physical realization of the logical package locations per platform.
//! Must agree with `nana_ui_platform::ApplicationPaths`, which resolves the
//! same locations at run time (checked by a test).

use std::path::{Path, PathBuf};

use nana_package::manifest::ManifestLayout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPlatform {
    Windows,
    Macos,
    Linux,
}

impl TargetPlatform {
    pub fn from_triple(triple: &str) -> Result<Self, String> {
        if triple.contains("windows") {
            Ok(Self::Windows)
        } else if triple.contains("apple-darwin") {
            Ok(Self::Macos)
        } else if triple.contains("linux") && !triple.contains("android") {
            Ok(Self::Linux)
        } else {
            Err(format!(
                "target `{triple}` has no packaging backend (windows, apple-darwin, linux)"
            ))
        }
    }

    pub fn host_triple() -> String {
        let arch = std::env::consts::ARCH;
        match std::env::consts::OS {
            "windows" => format!("{arch}-pc-windows-msvc"),
            "macos" => format!("{arch}-apple-darwin"),
            _ => format!("{arch}-unknown-linux-gnu"),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Windows, Self::Macos, Self::Linux]
            .into_iter()
            .find(|platform| platform.as_str() == text)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }

    /// Dynamic library extensions that must never sit in the application
    /// root.
    pub fn library_extensions(self) -> &'static [&'static str] {
        match self {
            Self::Windows => &["dll"],
            Self::Macos => &["dylib"],
            Self::Linux => &["so"],
        }
    }
}

/// Paths relative to the directory that holds the application (for macOS,
/// the directory containing `Name.app`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLayout {
    /// The application root: `App/` or `Name.app`.
    pub root: String,
    pub executable: String,
    pub runtime_bin: String,
    pub runtime_resources: String,
    pub runtime_plugins: String,
    pub runtime_tools: String,
    pub runtime_manifest: String,
}

impl PackageLayout {
    pub fn new(platform: TargetPlatform, app_name: &str, file_name: &str) -> Self {
        match platform {
            TargetPlatform::Windows | TargetPlatform::Linux => Self {
                root: file_name.to_owned(),
                executable: if platform == TargetPlatform::Windows {
                    format!("{file_name}.exe")
                } else {
                    file_name.to_owned()
                },
                runtime_bin: "runtime/bin".into(),
                runtime_resources: "runtime/resources".into(),
                runtime_plugins: "runtime/plugins".into(),
                runtime_tools: "runtime/tools".into(),
                runtime_manifest: "runtime/manifest".into(),
            },
            TargetPlatform::Macos => Self {
                root: format!("{app_name}.app"),
                executable: format!("Contents/MacOS/{file_name}"),
                runtime_bin: "Contents/Frameworks".into(),
                runtime_resources: "Contents/Resources".into(),
                runtime_plugins: "Contents/PlugIns".into(),
                runtime_tools: "Contents/Helpers".into(),
                runtime_manifest: "Contents/Resources/manifest".into(),
            },
        }
    }

    pub fn app_root(&self, out: &Path) -> PathBuf {
        out.join(&self.root)
    }

    pub fn join(root: &Path, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }

    pub fn manifest_layout(
        &self,
        root_exceptions: Vec<nana_package::manifest::RootException>,
    ) -> ManifestLayout {
        ManifestLayout {
            executable: self.executable.clone(),
            runtime_bin: self.runtime_bin.clone(),
            runtime_resources: self.runtime_resources.clone(),
            runtime_plugins: self.runtime_plugins.clone(),
            runtime_tools: self.runtime_tools.clone(),
            runtime_manifest: self.runtime_manifest.clone(),
            root_exceptions,
        }
    }

    /// Recover the layout from a manifest found inside `app_root`.
    pub fn from_manifest(root: &str, layout: &ManifestLayout) -> Self {
        Self {
            root: root.to_owned(),
            executable: layout.executable.clone(),
            runtime_bin: layout.runtime_bin.clone(),
            runtime_resources: layout.runtime_resources.clone(),
            runtime_plugins: layout.runtime_plugins.clone(),
            runtime_tools: layout.runtime_tools.clone(),
            runtime_manifest: layout.runtime_manifest.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_platform::{
        ApplicationIdentity, ApplicationLocation, ApplicationPaths, PathEnvironment, PathPlatform,
    };
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    /// The packager puts files exactly where the runtime looks for them.
    #[test]
    fn layout_agrees_with_application_paths() {
        let identity = ApplicationIdentity::new("dev.nana.fixture", "Fixture", "1.0.0");
        for (target, platform, base) in [
            (TargetPlatform::Windows, PathPlatform::Windows, "C:/Games"),
            (TargetPlatform::Linux, PathPlatform::Linux, "/opt/games"),
            (TargetPlatform::Macos, PathPlatform::MacOs, "/Applications"),
        ] {
            let layout = PackageLayout::new(target, "Fixture", "Fixture");
            let root = Path::new(base).join(&layout.root);
            let vars: BTreeMap<String, OsString> = [
                ("HOME", "/home/u"),
                ("APPDATA", "C:/Users/u/AppData/Roaming"),
                ("LOCALAPPDATA", "C:/Users/u/AppData/Local"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), OsString::from(v)))
            .collect();
            let env = PathEnvironment {
                platform,
                executable: PackageLayout::join(&root, &layout.executable),
                vars,
                android_files_dir: None,
                android_cache_dir: None,
                portable_marker: false,
            };
            let paths = ApplicationPaths::resolve_with(&identity, &env).unwrap();
            for (location, relative) in [
                (ApplicationLocation::RuntimeBin, &layout.runtime_bin),
                (
                    ApplicationLocation::RuntimeResources,
                    &layout.runtime_resources,
                ),
                (ApplicationLocation::RuntimePlugins, &layout.runtime_plugins),
                (ApplicationLocation::RuntimeTools, &layout.runtime_tools),
                (
                    ApplicationLocation::RuntimeManifest,
                    &layout.runtime_manifest,
                ),
            ] {
                assert_eq!(
                    paths.get(location),
                    PackageLayout::join(&root, relative),
                    "{target:?} {location:?}"
                );
            }
            assert_eq!(paths.app_root(), root);
        }
    }

    #[test]
    fn triples() {
        assert_eq!(
            TargetPlatform::from_triple("x86_64-pc-windows-msvc"),
            Ok(TargetPlatform::Windows)
        );
        assert_eq!(
            TargetPlatform::from_triple("aarch64-apple-darwin"),
            Ok(TargetPlatform::Macos)
        );
        assert_eq!(
            TargetPlatform::from_triple("x86_64-unknown-linux-gnu"),
            Ok(TargetPlatform::Linux)
        );
        assert!(TargetPlatform::from_triple("aarch64-linux-android").is_err());
    }
}
