//! Dev-loop configuration: what to watch, what to read, and how long to wait.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a watched tree must stay quiet before a batch is released.
///
/// Covers an editor's write-truncate-rename and a bundler emitting several files
/// as separate writes.
const DEFAULT_QUIET_PERIOD: Duration = Duration::from_millis(120);

/// Directory names never worth watching. A recursive watch on a Vite project
/// without this exhausts the inotify watch limit on Linux before it sees a
/// single useful event.
pub(crate) const IGNORED_DIRECTORIES: &[&str] = &["node_modules", "target", ".git"];

/// Which paths the dev loop watches, reads, and treats as hot-swappable CSS.
#[derive(Debug, Clone)]
pub struct DevConfig {
    artifact: Option<PathBuf>,
    jail: PathBuf,
    watch: Vec<PathBuf>,
    css: Vec<PathBuf>,
    quiet_period: Duration,
}

impl DevConfig {
    /// Watch the JS tier: `artifact` is the bundle handed to the engine, and its
    /// parent directory becomes both the jail root and the first watch root.
    ///
    /// Point this at the bundler's output, not at the sources — the host reads
    /// the built artifact, so a `.vue` edit is only interesting once the bundler
    /// has turned it into new bytes on disk.
    pub fn new(artifact: impl Into<PathBuf>) -> Self {
        let artifact = artifact.into();
        let root = artifact
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Self {
            artifact: Some(artifact),
            jail: root.clone(),
            watch: vec![root],
            css: Vec::new(),
            quiet_period: DEFAULT_QUIET_PERIOD,
        }
    }

    /// Watch the Rust L3 tier: `root` is the source tree whose changes should
    /// trigger a rebuild. There is no artifact to read — the new code arrives as
    /// a new binary.
    pub fn new_rust(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            artifact: None,
            jail: root.clone(),
            watch: vec![root],
            css: Vec::new(),
            quiet_period: DEFAULT_QUIET_PERIOD,
        }
    }

    /// Add a recursive watch root. Ignored directories still apply.
    #[must_use]
    pub fn watch(mut self, root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        if !self.watch.contains(&root) {
            self.watch.push(root);
        }
        self
    }

    /// Register a stylesheet for the CSS fast path.
    ///
    /// Only registered paths take it. An unregistered `.css` edit is treated as
    /// a full reload, because a stylesheet the host never loaded under a key has
    /// nothing to replace — and a bundler that inlines CSS into the JS bundle
    /// (Vite's IIFE default) will have rewritten the bundle anyway.
    #[must_use]
    pub fn css(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if !self.css.contains(&path) {
            self.css.push(path);
        }
        self
    }

    pub fn artifact(&self) -> Option<&Path> {
        self.artifact.as_deref()
    }

    pub fn jail_root(&self) -> &Path {
        &self.jail
    }

    pub fn watch_roots(&self) -> &[PathBuf] {
        &self.watch
    }

    pub fn css_paths(&self) -> &[PathBuf] {
        &self.css
    }

    pub const fn quiet(&self) -> Duration {
        self.quiet_period
    }
}

/// Skip build output, VCS metadata, dependency trees, and editor dotfiles.
///
/// Only the part of `path` *below* `root` is inspected. Testing the whole path
/// would refuse to watch any project that happens to live under a directory
/// named `target` or under a dotted parent — including this crate's own
/// `CARGO_TARGET_TMPDIR` scratch space.
pub(crate) fn is_ignored(path: &Path, root: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().any(|component| {
        let Some(name) = component.as_os_str().to_str() else {
            return false;
        };
        IGNORED_DIRECTORIES.contains(&name) || (name.starts_with('.') && name.len() > 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_directory_becomes_the_jail_and_the_first_watch_root() {
        let config = DevConfig::new("web/dist/app.iife.js");
        assert_eq!(config.jail_root(), Path::new("web/dist"));
        assert_eq!(config.watch_roots(), [PathBuf::from("web/dist")]);
        assert_eq!(config.artifact(), Some(Path::new("web/dist/app.iife.js")));
    }

    #[test]
    fn a_bare_artifact_filename_falls_back_to_the_current_directory() {
        let config = DevConfig::new("app.iife.js");
        assert_eq!(config.jail_root(), Path::new("."));
    }

    #[test]
    fn watch_and_css_roots_deduplicate() {
        let config = DevConfig::new("web/dist/app.iife.js")
            .watch("web/dist")
            .watch("assets")
            .css("web/dist/app.css")
            .css("web/dist/app.css");
        assert_eq!(
            config.watch_roots(),
            [PathBuf::from("web/dist"), PathBuf::from("assets")]
        );
        assert_eq!(config.css_paths(), [PathBuf::from("web/dist/app.css")]);
    }

    #[test]
    fn the_rust_tier_has_no_artifact_to_read() {
        let config = DevConfig::new_rust("src");
        assert!(config.artifact().is_none());
        assert_eq!(config.watch_roots(), [PathBuf::from("src")]);
    }

    #[test]
    fn build_output_and_dotfiles_are_ignored_at_any_depth_below_the_root() {
        let root = Path::new("web");
        assert!(is_ignored(Path::new("web/node_modules/vue/index.js"), root));
        assert!(is_ignored(Path::new("web/dist/target/app"), root));
        assert!(is_ignored(Path::new("web/.git/HEAD"), root));
        assert!(is_ignored(Path::new("web/.vite/deps/vue.js"), root));
        assert!(!is_ignored(Path::new("web/dist/app.iife.js"), root));
    }

    #[test]
    fn an_ignored_name_above_the_watch_root_does_not_disqualify_the_project() {
        // A project checked out under `~/.local/work/target/app` is watchable;
        // only what the developer asked to watch is filtered.
        let root = Path::new("/home/dev/.local/target/app");
        assert!(!is_ignored(
            Path::new("/home/dev/.local/target/app/dist/a.js"),
            root
        ));
        assert!(is_ignored(
            Path::new("/home/dev/.local/target/app/node_modules/vue.js"),
            root
        ));
    }
}
