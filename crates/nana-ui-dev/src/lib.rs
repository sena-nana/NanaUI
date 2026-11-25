#![recursion_limit = "256"]

//! Development-only hot reload support for NanaUI applications.
//!
//! This crate owns what is the same for every tier: a filesystem watcher, a
//! debounce that survives an editor's write-truncate-rename, and a jailed read
//! that skips a half-written file. Deciding *what a reload does* stays with the
//! application that owns the state.
//!
//! What a reload can actually reach, and why Rust code cannot be swapped into a
//! live process, is in `docs/reference/hot-reload.md`; [`restart`] carries the short
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

use std::path::PathBuf;

pub use config::DevConfig;
pub use restart::{DevHandoff, HANDOFF_ENV, RebuildCommand, RebuildOutcome, relaunch};
pub use watch::DevWatcher;
#[cfg(feature = "templates")]
pub use wiring::{
    TemplateEvent, TemplateStyle, TemplateText, watch_template_events, watch_templates,
};
pub use wiring::{request_relaunch, restored_handoff, run_with_restart, watch_and_rebuild};

/// What one settled filesystem batch asks the running application to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadRequest {
    /// Rebuild the binary and restart into it.
    Full,
    /// A `.vue` view under a directory registered with
    /// [`DevConfig::templates`] changed.
    Template { path: PathBuf },
}

/// Outcome of a rebuild, handed to a running L3 program.
///
/// Applications convert this into their own `RuntimeProgram::Message`, which is
/// why [`watch_and_rebuild`] requires `Message: From<DevSignal>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevSignal {
    /// The binary was rebuilt; the process should hand off its geometry and
    /// state, then exit for the outer loop to re-exec.
    Rebuilt,
    /// A rebuild failed. Carries the rendered diagnostics so the application can
    /// surface them instead of leaving the developer with a stale window.
    BuildFailed(String),
}

/// Why a dev-time filesystem operation failed.
///
/// Every variant is recoverable: the caller keeps running on what it already
/// has. A dev loop that turns one of these into a panic or a blank window is
/// worse than one that ignores the save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevError {
    /// The path could not be read or written.
    Unreadable(PathBuf),
}

impl std::fmt::Display for DevError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(path) => write!(formatter, "cannot access `{}`", path.display()),
        }
    }
}

impl std::error::Error for DevError {}
