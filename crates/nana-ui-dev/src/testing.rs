//! Scratch directories for the crate's own tests.
//!
//! `CARGO_TARGET_TMPDIR` is only defined for integration tests and benches, and
//! these are unit tests (the debounce and jail helpers are private). Prefer it
//! when it is there so scratch state lands under `target/`, and fall back to the
//! system temp directory otherwise.

use std::path::PathBuf;

pub(crate) fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base
        .join(concat!(env!("CARGO_PKG_NAME"), "-tests"))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    // Canonicalize so jail comparisons see the same form the watcher would:
    // macOS resolves /var and /tmp through symlinks.
    std::fs::canonicalize(&dir).expect("canonicalize scratch dir")
}
