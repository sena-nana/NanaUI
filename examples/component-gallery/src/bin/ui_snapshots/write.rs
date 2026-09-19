use std::path::Path;

pub use nana_ui_devtools::offscreen::{Size, read_png, write_png as png};

/// Write a text baseline, creating the directory first.
///
/// Always `\n`: a baseline whose bytes depend on the checkout's line-ending
/// policy is a baseline that fails on the other platform for no reason.
pub fn text(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body.replace("\r\n", "\n"))
}
