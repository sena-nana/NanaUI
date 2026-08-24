//! Clipboard host boundary.
//!
//! Desktop uses the OS clipboard via `arboard` ([`OsClipboard`]). Android does
//! not compile arboard; [`default_shared_clipboard`] installs
//! [`UnsupportedClipboard`]. Keep `PlatformCapabilities::clipboard = false`
//! until a real Android clipboard path exists.

use std::sync::{Arc, Mutex};

/// Minimal clipboard contract for Nana hosts — never System WebView.
pub trait ClipboardHost: Send {
    fn read_text(&mut self) -> Option<String>;
    fn write_text(&mut self, text: &str) -> bool;
}

/// Shared clipboard handle installed into host API registries.
pub type SharedClipboardHost = Arc<Mutex<Box<dyn ClipboardHost>>>;

/// Wrap any [`ClipboardHost`] for host-op registration.
pub fn shared_clipboard<C: ClipboardHost + 'static>(clipboard: C) -> SharedClipboardHost {
    Arc::new(Mutex::new(Box::new(clipboard)))
}

/// Platform default: OS clipboard on desktop; unsupported on Android.
pub fn default_shared_clipboard() -> SharedClipboardHost {
    #[cfg(target_os = "android")]
    {
        shared_clipboard(UnsupportedClipboard)
    }
    #[cfg(not(target_os = "android"))]
    {
        shared_clipboard(OsClipboard::new())
    }
}

/// Always-unavailable clipboard (Android MVP).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedClipboard;

impl ClipboardHost for UnsupportedClipboard {
    fn read_text(&mut self) -> Option<String> {
        None
    }

    fn write_text(&mut self, _text: &str) -> bool {
        false
    }
}

/// In-process clipboard for unit tests (does not touch the OS pasteboard).
#[derive(Debug, Default, Clone)]
pub struct MemoryClipboard {
    text: Option<String>,
}

impl MemoryClipboard {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ClipboardHost for MemoryClipboard {
    fn read_text(&mut self) -> Option<String> {
        self.text.clone()
    }

    fn write_text(&mut self, text: &str) -> bool {
        self.text = Some(text.to_owned());
        true
    }
}

/// Desktop OS clipboard (`arboard`). Construction never panics; unavailable
/// backends report failed reads/writes. Not compiled on Android.
#[cfg(not(target_os = "android"))]
pub struct OsClipboard {
    inner: Option<arboard::Clipboard>,
}

#[cfg(not(target_os = "android"))]
impl std::fmt::Debug for OsClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsClipboard")
            .field("available", &self.inner.is_some())
            .finish()
    }
}

#[cfg(not(target_os = "android"))]
impl OsClipboard {
    pub fn new() -> Self {
        Self {
            inner: arboard::Clipboard::new().ok(),
        }
    }

    pub fn is_available(&self) -> bool {
        self.inner.is_some()
    }
}

#[cfg(not(target_os = "android"))]
impl Default for OsClipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_os = "android"))]
impl ClipboardHost for OsClipboard {
    fn read_text(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }

    fn write_text(&mut self, text: &str) -> bool {
        match self.inner.as_mut() {
            Some(clipboard) => clipboard.set_text(text.to_owned()).is_ok(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_clipboard_is_inert() {
        let mut clip = UnsupportedClipboard;
        assert!(clip.read_text().is_none());
        assert!(!clip.write_text("nana"));
    }

    #[test]
    fn memory_clipboard_roundtrip() {
        let mut clip = MemoryClipboard::new();
        assert!(clip.read_text().is_none());
        assert!(clip.write_text("hello"));
        assert_eq!(clip.read_text().as_deref(), Some("hello"));
        assert!(clip.write_text(""));
        assert_eq!(clip.read_text().as_deref(), Some(""));
    }

    #[test]
    #[cfg(not(target_os = "android"))]
    fn os_clipboard_roundtrip_when_available() {
        let mut clip = OsClipboard::new();
        if !clip.is_available() {
            // Headless / locked-down environments may lack a pasteboard.
            return;
        }
        let marker = format!("nana-ui-clipboard-{}-{}", std::process::id(), "probe");
        let previous = clip.read_text();
        assert!(
            clip.write_text(&marker),
            "desktop OS clipboard write must succeed when available"
        );
        assert_eq!(
            clip.read_text().as_deref(),
            Some(marker.as_str()),
            "desktop OS clipboard read must return the written text"
        );
        if let Some(prev) = previous {
            let _ = clip.write_text(&prev);
        }
    }
}
