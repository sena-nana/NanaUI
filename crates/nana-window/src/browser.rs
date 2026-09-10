//! Host-owned native browsing content. Native handles never reach Runtime controls.

use raw_window_handle::HasWindowHandle;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrowserPolicy {
    /// Explicit application opt-in for http(s) navigation, including redirects.
    pub allow_web: bool,
}

impl BrowserPolicy {
    /// Applies to direct navigation, redirects, and links opening a new window.
    pub fn allows(&self, address: &str) -> bool {
        if address == "about:blank" {
            return true;
        }
        self.allow_web
            && url::Url::parse(address).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserCommand {
    Navigate(String),
    Back,
    Forward,
    Reload,
    Stop,
    Focus,
    Capture,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrowserState {
    pub attached: bool,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BrowserEvent {
    State(BrowserState),
    Captured(Vec<u8>),
    CaptureFailed(String),
}

/// An event carries the command revision at its origin, including asynchronous captures.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserCompletion {
    pub revision: u64,
    pub event: BrowserEvent,
}

/// Coordinates in the owning window's logical, top-left coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BrowserRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[cfg(target_os = "macos")]
#[path = "browser/macos.rs"]
mod platform;

/// One browser belongs to one host window and is destroyed before that window.
pub struct NativeBrowser {
    #[cfg(target_os = "macos")]
    inner: platform::MacBrowser,
}

impl NativeBrowser {
    pub fn new<W: HasWindowHandle + ?Sized>(
        window: &W,
        policy: BrowserPolicy,
        wake: Box<dyn Fn()>,
    ) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                inner: platform::MacBrowser::new(window, policy, wake)?,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (window, policy, wake);
            Err("此平台暂不支持内嵌浏览".into())
        }
    }

    pub fn set_geometry(&mut self, bounds: BrowserRect, clip: BrowserRect, visible: bool) {
        #[cfg(target_os = "macos")]
        self.inner.set_geometry(bounds, clip, visible);
        #[cfg(not(target_os = "macos"))]
        let _ = (bounds, clip, visible);
    }

    pub fn command(
        &mut self,
        revision: u64,
        command: Option<&BrowserCommand>,
    ) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            self.inner.command(revision, command)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (revision, command);
            Err("此平台暂不支持内嵌浏览".into())
        }
    }

    pub fn take_events(&mut self) -> Vec<BrowserCompletion> {
        #[cfg(target_os = "macos")]
        {
            self.inner.take_events()
        }
        #[cfg(not(target_os = "macos"))]
        {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_policy_is_explicit_and_applies_to_parsed_web_addresses() {
        let disabled = BrowserPolicy::default();
        assert!(disabled.allows("about:blank"));
        assert!(!disabled.allows("https://example.com"));
        let enabled = BrowserPolicy { allow_web: true };
        assert!(enabled.allows("HTTPS://example.com/a"));
        assert!(enabled.allows("http://localhost:3000"));
        for url in [
            "file:///tmp/private",
            "javascript:alert(1)",
            "data:text/html,hello",
            "https://",
            "https://user:pass@example.com",
        ] {
            assert!(!enabled.allows(url), "{url}");
        }
    }
}
