//! Development-only hot reload support for NanaUI applications.
//!
//! This crate owns what is the same for every tier: a filesystem watcher, a
//! debounce that survives an editor's write-truncate-rename, and a jailed read
//! that refuses a half-written file. Deciding *what a reload does* stays with
//! the tier that owns the state — `nana_ui_vue::dev` for the Vue tiers, the
//! application itself for Rust L3.
//!
//! What each tier can actually reach, and why Rust code cannot be swapped into
//! a live process, is in `docs/hot-reload.md`; [`restart`] carries the short
//! version at the point it matters.
//!
//! # This crate cannot ship
//!
//! `[profile.dist]` inherits `release`, so `debug_assertions` is off in every
//! packaged build and the `compile_error!` below fires. A product crate that
//! grows a dependency on `nana-ui-dev` fails to build rather than shipping a
//! filesystem watcher to users. There is deliberately no escape hatch.

#[cfg(not(debug_assertions))]
compile_error!(
    "nana-ui-dev is development-only and must not reach a release or dist build. \
     Keep the dev entry point in its own binary behind `required-features`, and \
     keep `nana-ui-dev` out of the product crate's dependencies."
);

mod config;
mod restart;
#[cfg(test)]
mod testing;
mod watch;
mod wiring;

use std::path::{Path, PathBuf};

pub use config::DevConfig;
pub use restart::{DevHandoff, HANDOFF_ENV, RebuildCommand, RebuildOutcome, relaunch};
pub use watch::DevWatcher;
#[cfg(feature = "vue")]
pub use wiring::watch_and_reload;
pub use wiring::{request_relaunch, restored_handoff, run_with_restart, watch_and_rebuild};

use nana_js_engine::RuntimeArtifact;

/// Size cap for a JS artifact read from disk.
///
/// Deliberately far above `nana_ui_core::MAX_LOCAL_URL_BYTES` (8 MiB, sized for
/// fonts and images): an unminified dev bundle with an inline source map clears
/// that on a mid-sized application.
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

/// What one settled filesystem batch asks the running application to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadRequest {
    /// Re-read the artifact and remount the tree. Browser-refresh semantics:
    /// window, position, GPU device and surface survive; page state does not.
    Full,
    /// Replace one keyed stylesheet in place. No node is created or destroyed,
    /// so node ids, focus, scroll offsets and in-flight animations all survive.
    Css { path: PathBuf },
}

/// Outcome of a rebuild, handed to a running L3 program.
///
/// Applications convert this into their own `RuntimeProgram::Message`, which is
/// why [`watch_and_rebuild`] requires `Message: From<DevSignal>`. The Vue tier
/// does not use this type: new code arrives there as new bytes, not as a new
/// binary, so its watcher dispatches a `nana_ui_vue::dev::DevReload` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevSignal {
    /// The binary was rebuilt; the process should hand off its geometry and
    /// state, then exit for the outer loop to re-exec.
    Rebuilt,
    /// A rebuild failed. Carries the rendered diagnostics so the application can
    /// surface them instead of leaving the developer with a stale window.
    BuildFailed(String),
}

/// Why a dev-time filesystem read could not produce an artifact.
///
/// Every variant is recoverable: the caller keeps running on the artifact it
/// already has. A dev loop that turns one of these into a panic or a blank
/// window is worse than one that ignores the save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevError {
    /// The path did not canonicalize inside the jail, exceeded
    /// [`MAX_ARTIFACT_BYTES`], or could not be read.
    Unreadable(PathBuf),
    /// The file exists but is empty — almost always a half-written save caught
    /// between truncate and write.
    Empty(PathBuf),
    /// The bytes are not UTF-8, so they are not a source artifact.
    NotUtf8(PathBuf),
    /// The path is not representable as UTF-8 and cannot be passed to the jail.
    NonUtf8Path(PathBuf),
}

impl std::fmt::Display for DevError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(path) => {
                write!(
                    formatter,
                    "cannot read `{}` inside the dev jail",
                    path.display()
                )
            }
            Self::Empty(path) => write!(
                formatter,
                "`{}` is empty; treating it as a half-written save",
                path.display()
            ),
            Self::NotUtf8(path) => write!(formatter, "`{}` is not UTF-8 source", path.display()),
            Self::NonUtf8Path(path) => {
                write!(formatter, "path `{}` is not valid UTF-8", path.display())
            }
        }
    }
}

impl std::error::Error for DevError {}

/// Read a UTF-8 source file from disk, jailed under `jail`.
///
/// Returns `(canonical path, contents)`. The canonical path is the identity
/// everything downstream keys on: `nana_ui_core::stylesheet_base_from_href`
/// derives an artifact's `@import` base from it, and a stylesheet reload has to
/// name the same key the original `injectStylesheet(css, href)` recorded.
///
/// An empty file is an error rather than empty content: it is nearly always a
/// save caught between truncate and write, and handing it to the engine would
/// replace a working app with a blank one.
pub fn read_within_jail(path: &Path, jail: &Path) -> Result<(String, String), DevError> {
    let href = path
        .to_str()
        .ok_or_else(|| DevError::NonUtf8Path(path.to_path_buf()))?;
    let (bytes, canonical) =
        nana_ui_core::read_file_within_jail(href, None, jail, MAX_ARTIFACT_BYTES)
            .ok_or_else(|| DevError::Unreadable(path.to_path_buf()))?;
    if bytes.is_empty() {
        return Err(DevError::Empty(canonical));
    }
    let source = String::from_utf8(bytes).map_err(|_| DevError::NotUtf8(canonical.clone()))?;
    Ok((canonical.to_string_lossy().into_owned(), source))
}

/// [`read_within_jail`] as a `RuntimeArtifact`, named with its canonical path.
pub fn artifact_from_path(path: &Path, jail: &Path) -> Result<RuntimeArtifact, DevError> {
    let (name, source) = read_within_jail(path, jail)?;
    Ok(RuntimeArtifact::from_source(name, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch;
    use std::io::Write;

    fn write(path: &Path, contents: &[u8]) {
        let mut file = std::fs::File::create(path).expect("create");
        file.write_all(contents).expect("write");
    }

    #[test]
    fn artifact_is_named_with_its_canonical_path() {
        let dir = scratch("artifact-name");
        let js = dir.join("app.iife.js");
        write(&js, b"globalThis.x = 1;");

        let artifact = artifact_from_path(&js, &dir).expect("artifact");
        assert_eq!(artifact.name, js.to_string_lossy());
        assert_eq!(artifact.source_utf8().expect("source"), "globalThis.x = 1;");
    }

    #[test]
    fn empty_file_is_reported_not_returned_as_an_artifact() {
        let dir = scratch("artifact-empty");
        let js = dir.join("app.iife.js");
        write(&js, b"");

        let error = artifact_from_path(&js, &dir).expect_err("empty is an error");
        assert!(matches!(error, DevError::Empty(_)), "got {error:?}");
    }

    #[test]
    fn a_path_outside_the_jail_is_refused() {
        let jail = scratch("artifact-jail");
        let outside = scratch("artifact-outside");
        let js = outside.join("app.iife.js");
        write(&js, b"globalThis.x = 1;");

        let error = artifact_from_path(&js, &jail).expect_err("outside the jail");
        assert!(matches!(error, DevError::Unreadable(_)), "got {error:?}");
    }

    #[test]
    fn missing_file_is_unreadable_rather_than_a_panic() {
        let dir = scratch("artifact-missing");
        let error = artifact_from_path(&dir.join("nope.js"), &dir).expect_err("missing");
        assert!(matches!(error, DevError::Unreadable(_)), "got {error:?}");
    }
}
