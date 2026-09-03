//! Platform input / IME / clipboard abstraction plus Android MVP flags.

#[cfg(feature = "clipboard")]
mod clipboard;
#[cfg(feature = "fetch")]
mod fetch;
mod ime;
mod input;
mod window;
#[cfg(feature = "ws")]
mod ws;

#[cfg(all(feature = "clipboard", not(target_os = "android")))]
pub use clipboard::OsClipboard;
#[cfg(feature = "clipboard")]
pub use clipboard::{
    ClipboardHost, MemoryClipboard, SharedClipboardHost, UnsupportedClipboard,
    default_shared_clipboard, shared_clipboard,
};
#[cfg(feature = "fetch")]
pub use fetch::{
    DEFAULT_FETCH_TIMEOUT, FetchCancellation, FetchError, FetchErrorKind, FetchHost, FetchPolicy,
    FetchRequest, FetchResponse, NativeFetchHost, SharedFetchHost, fetch_bytes_blocking,
    shared_fetch_host,
};
pub use ime::ImeEvent;
pub use input::{InputDisposition, InputEvent, InputModifiers, PointerPhase, PointerType};
pub use window::{
    DisplayBounds, TextInputPurpose, TextInputRequest, WindowCommand, WindowEvent, WindowGeometry,
    WindowIcon, WindowIconError, WindowId, WindowResizeEdge, WindowRole, WindowSettings,
    clamp_position_to_displays, clear_registered_application_icon, register_application_icon,
    resolve_window_icon, window_resize_edge,
};
#[cfg(feature = "ws")]
pub use ws::{
    SharedWebSocketHost, SocketPolicy, WebSocketHost, WsError, WsErrorKind, WsEvent, WsMessage,
    WsOpenRequest, WsSink,
};

/// Experimental Android host capability flags. They are intentionally absent
/// from the default desktop/framework API until Android becomes a supported
/// NanaUI product target.
#[cfg(feature = "experimental-android")]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCapabilities {
    pub ime: bool,
    pub clipboard: bool,
    /// Host presents via wgpu Surface (Vulkan on Android ARM64).
    pub vulkan_surface: bool,
    /// Rust-owned JS engine (V8) — never System WebView.
    pub rust_js_engine: bool,
    /// Nana `DesktopShell` paint wired on this target.
    pub desktop_shell: bool,
    /// Shell chrome band plan (title / regions) available.
    pub shell_chrome_bands: bool,
    /// Solid-color scissor fill for chrome bands.
    pub shell_chrome_fill: bool,
    /// Primary-region control-slot geometry reserved (not full DesktopShell).
    pub control_slot: bool,
    /// Runtime controls (Text + Input + Switch + Button) can paint into the slot.
    pub control_widget: bool,
    /// NativeActivity (or host) pointer events route into the slot control.
    pub control_input: bool,
}

#[cfg(feature = "experimental-android")]
impl PlatformCapabilities {
    /// Android ARM64 MVP: Surface + Rust JS engine + slot controls/input.
    ///
    /// `ime` stays false on NativeActivity (no InputConnection); KeyEvent text
    /// is a separate path under `control_input`, not a soft-IME claim.
    /// `clipboard` stays false until a real Android clipboard backend exists.
    pub const fn android_mvp() -> Self {
        Self {
            ime: false,
            clipboard: false,
            vulkan_surface: true,
            rust_js_engine: true,
            desktop_shell: false,
            shell_chrome_bands: true,
            shell_chrome_fill: true,
            control_slot: true,
            control_widget: true,
            control_input: true,
        }
    }
}

#[cfg(all(test, feature = "experimental-android"))]
mod tests {
    use super::*;

    #[test]
    fn android_capabilities_do_not_claim_unwired_native_services() {
        let caps = PlatformCapabilities::android_mvp();
        assert!(!caps.ime);
        assert!(!caps.clipboard);
        assert!(!caps.desktop_shell);
    }

    /// Shared L1 fields via [`Default`] — Android does not fork `LayoutStyle`.
    #[test]
    fn experimental_android_shares_l1_layout_fields() {
        let style = nana_ui_core::LayoutStyle::default();
        let _pointer_events: Option<nana_ui_core::PointerEventsSpec> = style.pointer_events;
        let _transform_3d: Option<nana_ui_core::PaintMat4> = style.transform_3d;
        let _logical_padding: nana_ui_core::LogicalInlineEdges = style.logical_padding;
        let _logical_margin: nana_ui_core::LogicalInlineEdges = style.logical_margin;
        let _logical_inset: nana_ui_core::LogicalInlineEdges = style.logical_inset;
    }

    /// Compile cfg: Android is an experimental host OS, not a product UI target.
    #[test]
    fn android_is_not_a_product_os_cfg() {
        #[cfg(target_os = "android")]
        {
            let _ = SurfacePhase::Pending;
            assert!(!PlatformCapabilities::android_mvp().desktop_shell);
        }
    }
}

/// Experimental Android surface lifecycle owned by the Android host crate.
#[cfg(feature = "experimental-android")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfacePhase {
    /// No native window yet (activity started / paused without window).
    Pending,
    /// `ANativeWindow` available — safe to create wgpu Surface.
    Ready,
    /// Window torn down — drop Surface before the next Ready.
    Destroyed,
}
