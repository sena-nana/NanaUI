//! Build-side Nana packager (Issue #226).
//!
//! Turns an already built executable, a resource tree and
//! `nana-package.toml` into a distributable application: packs, platform
//! layout, package manifest, Steam depot metadata, and a validation report.
//! See `docs/packaging.md`.

pub mod cache;
pub mod config;
pub mod delta;
pub mod layout;
pub mod macos;
pub mod pack_build;
pub mod package;
pub mod plan;
pub mod secrets;
pub mod sign;
pub mod steam;
mod util;
pub mod validate;
