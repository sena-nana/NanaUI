//! Headless web pages whose frames the application routes itself.
//!
//! Unlike [`crate::NativeBrowserRequest`], a web surface is not anchored to a
//! node or a window: the host keeps one engine instance per requested id and
//! hands every captured frame to the request's [`WebFrameSink`] on a
//! background thread. Where the frames go — a HostTexture slot, a renderer of
//! the application's own — is the application's decision.

pub use nana_window::{
    MAX_WEB_SURFACE_EDGE, MAX_WEB_SURFACE_FPS, WebFrame, WebFrameSink, WebSurfaceCommand,
    WebSurfaceDesc, WebSurfaceEvent, web_surface_support,
};

use crate::BrowserPolicy;

#[derive(Clone)]
pub struct WebSurfaceRequest {
    pub id: String,
    pub policy: BrowserPolicy,
    /// Size, scale, transparency and capture rate; changes apply in place.
    pub desc: WebSurfaceDesc,
    /// The page a new engine instance loads. Instances are never recreated to
    /// replay Reload or ShowWindow.
    pub restore_url: String,
    /// Monotonic command identity; a command runs once, when its revision is new.
    pub revision: u64,
    pub command: Option<WebSurfaceCommand>,
    /// Receives frames on a background thread. Replacing the sink (a
    /// different `Arc`) recreates the engine instance.
    pub frames: WebFrameSink,
}

impl std::fmt::Debug for WebSurfaceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSurfaceRequest")
            .field("id", &self.id)
            .field("policy", &self.policy)
            .field("desc", &self.desc)
            .field("restore_url", &self.restore_url)
            .field("revision", &self.revision)
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebSurfaceNotice {
    pub id: String,
    /// The command revision current when the event was produced.
    pub revision: u64,
    pub event: WebSurfaceEvent,
}
