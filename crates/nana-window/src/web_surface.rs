//! Headless web content rendered into frames for host-owned GPU content.
//!
//! A [`WebSurface`] owns one platform web engine instance that is never part
//! of a host window's view tree. Its page is laid out at
//! [`WebSurfaceDesc::size`] CSS pixels and captured at most
//! [`WebSurfaceDesc::max_fps`] times a second; each capture reaches the
//! application's [`WebFrameSink`] on a background thread, so the UI thread
//! never converts or queues pixels. The application decides where frames go
//! (a HostTexture slot, a compositor of its own).
//!
//! [`WebSurfaceCommand::ShowWindow`] moves the same page into an ordinary
//! native window for direct interaction (sign-in, clicking through a widget's
//! settings); cookies and page state stay, and frames keep flowing while it is
//! open. Closing that window returns the page offscreen and reports
//! [`WebSurfaceEvent::WindowClosed`].

use std::sync::Arc;

use crate::{BrowserPolicy, BrowserState};

/// Receives captured frames on a background thread. Keep it cheap: a slow
/// sink drops later captures instead of queueing them.
pub type WebFrameSink = Arc<dyn Fn(WebFrame) + Send + Sync>;

/// Wakes the host after an event was queued. Called from any thread.
pub type WebSurfaceWake = Arc<dyn Fn() + Send + Sync>;

/// The largest edge a surface lays out or captures, in pixels.
const MAX_WEB_SURFACE_EDGE: u32 = 4096;
/// The highest capture rate a surface accepts.
const MAX_WEB_SURFACE_FPS: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WebSurfaceDesc {
    /// Page viewport in CSS pixels. Frames are this size times `scale`.
    pub size: [u32; 2],
    /// Device pixel ratio the page renders at (1.0 = one frame pixel per CSS pixel).
    pub scale: f32,
    /// Leave the page background transparent instead of painting it white.
    pub transparent: bool,
    /// Capture rate ceiling; 0 pauses capture without unloading the page.
    pub max_fps: u32,
}

impl Default for WebSurfaceDesc {
    fn default() -> Self {
        Self {
            size: [1280, 720],
            scale: 1.0,
            transparent: true,
            max_fps: 30,
        }
    }
}

impl WebSurfaceDesc {
    /// The same description with every field inside the supported range.
    pub fn clamped(self) -> Self {
        let scale = if self.scale.is_finite() {
            self.scale.clamp(0.25, 4.0)
        } else {
            1.0
        };
        let max_edge = (MAX_WEB_SURFACE_EDGE as f32 / scale).floor().max(1.0) as u32;
        Self {
            size: self.size.map(|edge| edge.clamp(1, max_edge)),
            scale,
            transparent: self.transparent,
            max_fps: self.max_fps.min(MAX_WEB_SURFACE_FPS),
        }
    }

    /// Captured frame size in pixels of a [`Self::clamped`] description.
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    pub(crate) fn frame_size(&self) -> [u32; 2] {
        self.size
            .map(|edge| ((edge as f32 * self.scale).round() as u32).clamp(1, MAX_WEB_SURFACE_EDGE))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSurfaceCommand {
    Navigate(String),
    Reload,
    /// Show the page in a native window the user can interact with.
    ShowWindow {
        title: String,
    },
    /// Return the page offscreen; frames keep flowing.
    HideWindow,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WebSurfaceEvent {
    State(BrowserState),
    /// The interaction window closed (by the user or [`WebSurfaceCommand::HideWindow`]).
    WindowClosed,
}

/// An event carries the command revision current when it was produced.
#[derive(Debug, Clone, PartialEq)]
pub struct WebSurfaceCompletion {
    pub revision: u64,
    pub event: WebSurfaceEvent,
}

/// One captured page image.
#[derive(Clone)]
pub struct WebFrame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8, sRGB-encoded, alpha premultiplied in sRGB space.
    pub rgba: Arc<[u8]>,
    /// Increases by one for every frame this surface delivers.
    pub sequence: u64,
}

impl std::fmt::Debug for WebFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("sequence", &self.sequence)
            .finish_non_exhaustive()
    }
}

/// Whether this build has a headless web engine at all. A supported platform
/// may still fail at run time (an old OS, a missing WebView2 Runtime); that
/// failure is returned by [`WebSurface::new`] or reported as a state error.
pub const fn web_surface_support() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

#[cfg(target_os = "macos")]
#[path = "web_surface/macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "web_surface/windows.rs"]
mod platform;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub(super) struct PlatformSurface;

    impl PlatformSurface {
        pub(super) fn new(
            _: BrowserPolicy,
            _: WebSurfaceDesc,
            _: WebFrameSink,
            _: WebSurfaceWake,
        ) -> Result<Self, String> {
            Err("此平台不支持网页画面".into())
        }
        pub(super) fn configure(&mut self, _: WebSurfaceDesc) {}
        pub(super) fn command(
            &mut self,
            _: u64,
            _: Option<&WebSurfaceCommand>,
        ) -> Result<(), String> {
            Ok(())
        }
        pub(super) fn take_events(&mut self) -> Vec<WebSurfaceCompletion> {
            Vec::new()
        }
    }
}

/// A headless page. On macOS create and drive it on the main thread; on
/// Windows any one thread may own it (the engine runs on its own thread).
pub struct WebSurface {
    inner: platform::PlatformSurface,
}

impl WebSurface {
    pub fn new(
        policy: BrowserPolicy,
        desc: WebSurfaceDesc,
        frames: WebFrameSink,
        wake: WebSurfaceWake,
    ) -> Result<Self, String> {
        Ok(Self {
            inner: platform::PlatformSurface::new(policy, desc.clamped(), frames, wake)?,
        })
    }

    /// Apply a new size, scale, transparency or capture rate. The page stays loaded.
    pub fn configure(&mut self, desc: WebSurfaceDesc) {
        self.inner.configure(desc.clamped());
    }

    /// Run `command` under `revision`; `None` only re-publishes the current state.
    pub fn command(
        &mut self,
        revision: u64,
        command: Option<&WebSurfaceCommand>,
    ) -> Result<(), String> {
        self.inner.command(revision, command)
    }

    pub fn take_events(&mut self) -> Vec<WebSurfaceCompletion> {
        self.inner.take_events()
    }
}

/// Pending events of one surface: the latest state replaces older states,
/// everything else is kept in order. Closing drops queued and late events.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
#[derive(Default)]
pub(crate) struct SurfaceEvents {
    events: Vec<WebSurfaceCompletion>,
    closed: bool,
}

#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
impl SurfaceEvents {
    pub(crate) fn publish(&mut self, revision: u64, event: WebSurfaceEvent) {
        if self.closed {
            return;
        }
        if matches!(event, WebSurfaceEvent::State(_)) {
            self.events
                .retain(|old| !matches!(old.event, WebSurfaceEvent::State(_)));
        }
        self.events.push(WebSurfaceCompletion { revision, event });
    }

    pub(crate) fn take(&mut self) -> Vec<WebSurfaceCompletion> {
        std::mem::take(&mut self.events)
    }

    pub(crate) fn close(&mut self) {
        self.closed = true;
        self.events.clear();
    }
}

/// Throttles captures to a rate; shared by both backends.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub(crate) fn capture_interval(max_fps: u32) -> Option<std::time::Duration> {
    (max_fps > 0).then(|| std::time::Duration::from_secs_f64(1.0 / max_fps as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_is_clamped_to_supported_ranges() {
        let desc = WebSurfaceDesc {
            size: [0, 100_000],
            scale: f32::NAN,
            transparent: false,
            max_fps: 240,
        }
        .clamped();
        assert_eq!(desc.size, [1, MAX_WEB_SURFACE_EDGE]);
        assert_eq!(desc.scale, 1.0);
        assert_eq!(desc.max_fps, MAX_WEB_SURFACE_FPS);
        let retina = WebSurfaceDesc {
            size: [4000, 300],
            scale: 2.0,
            ..WebSurfaceDesc::default()
        };
        assert_eq!(retina.clamped().size, [2048, 300]);
        assert_eq!(retina.clamped().frame_size(), [4096, 600]);
    }

    #[test]
    fn latest_state_replaces_older_states_but_keeps_window_events() {
        let mut events = SurfaceEvents::default();
        events.publish(1, WebSurfaceEvent::State(BrowserState::default()));
        events.publish(1, WebSurfaceEvent::WindowClosed);
        events.publish(
            2,
            WebSurfaceEvent::State(BrowserState {
                title: "later".into(),
                ..Default::default()
            }),
        );
        let taken = events.take();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].event, WebSurfaceEvent::WindowClosed);
        assert_eq!(taken[1].revision, 2);
        assert!(events.take().is_empty());
    }

    #[test]
    fn closed_surfaces_drop_queued_and_late_events() {
        let mut events = SurfaceEvents::default();
        events.publish(1, WebSurfaceEvent::WindowClosed);
        events.close();
        events.publish(2, WebSurfaceEvent::State(BrowserState::default()));
        assert!(events.take().is_empty());
    }
}
