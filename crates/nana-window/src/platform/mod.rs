#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{apply, clear};
#[cfg(target_os = "macos")]
pub(crate) use macos::{apply, clear};
#[cfg(target_os = "windows")]
pub(crate) use windows::{apply, clear};
