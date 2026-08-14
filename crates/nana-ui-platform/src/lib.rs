//! Platform input / IME / clipboard abstraction plus Android MVP flags.

#[cfg(feature = "clipboard")]
mod clipboard;
#[cfg(feature = "fetch")]
mod fetch;
mod ime;
mod input;
mod window;

#[cfg(feature = "clipboard")]
pub use clipboard::{
    ClipboardHost, MemoryClipboard, OsClipboard, SharedClipboardHost, UnsupportedClipboard,
    default_shared_clipboard, shared_clipboard,
};
#[cfg(feature = "fetch")]
pub use fetch::{
    FetchCancellation, FetchError, FetchErrorKind, FetchHost, FetchPolicy, FetchRequest,
    FetchResponse, NativeFetchHost, SharedFetchHost, shared_fetch_host,
};
pub use ime::ImeEvent;
pub use input::{InputDisposition, InputEvent, InputModifiers, PointerPhase, PointerType};
pub use window::{
    TextInputPurpose, TextInputRequest, WindowCommand, WindowEvent, WindowGeometry, WindowId,
    WindowRole, WindowSettings,
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
    /// Rust-owned JS engine (QuickJS/V8) — never System WebView.
    pub rust_js_engine: bool,
    /// Nana Iced `DesktopShell` paint wired on this target.
    pub iced_shell: bool,
    /// Pre-Iced shell chrome band plan (title / regions) available.
    pub shell_chrome_bands: bool,
    /// Pre-Iced solid-color scissor fill for chrome bands.
    pub shell_chrome_fill: bool,
    /// Primary-region Iced control-slot geometry reserved (not full DesktopShell).
    pub iced_control_slot: bool,
    /// Iced Nana controls (Icon + Text + Input + Switch + Button) can paint into the Primary slot.
    pub iced_control_widget: bool,
    /// NativeActivity (or host) pointer events route into the slot control.
    pub iced_control_input: bool,
}

#[cfg(feature = "experimental-android")]
impl PlatformCapabilities {
    /// Android ARM64 MVP: Surface + Rust JS engine + slot controls/input.
    ///
    /// `ime` stays false on NativeActivity (no InputConnection); KeyEvent text
    /// is a separate path under `iced_control_input`, not a soft-IME claim.
    /// `clipboard` stays false until a real Android clipboard backend exists.
    pub const fn android_mvp() -> Self {
        Self {
            ime: false,
            clipboard: false,
            vulkan_surface: true,
            rust_js_engine: true,
            iced_shell: false,
            shell_chrome_bands: true,
            shell_chrome_fill: true,
            iced_control_slot: true,
            iced_control_widget: true,
            iced_control_input: true,
        }
    }
}

#[cfg(all(test, feature = "experimental-android"))]
mod tests {
    use super::*;

    #[test]
    fn android_capabilities_do_not_claim_unwired_native_services() {
        assert!(!PlatformCapabilities::android_mvp().ime);
        assert!(!PlatformCapabilities::android_mvp().clipboard);
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
