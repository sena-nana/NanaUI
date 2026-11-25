//! Platform input / IME / clipboard abstraction plus Android MVP flags.

pub mod android_assets;
mod clipboard;
mod fetch;
mod locale;
mod notification;
pub mod path;
mod paths;
mod persist;
mod persistence_worker;
pub use persistence_worker::{PersistenceCoordinator, PersistenceWork};
mod window;
mod window_shadow;
pub use window_shadow::{
    WindowShadow, WindowShadowBackend, WindowShadowFallback, WindowShadowOutcome,
    WindowShadowSource, WindowShadowStyle, WindowShadowWork, WindowVisualShape,
};
#[cfg(feature = "ws")]
mod ws;

#[cfg(all(feature = "clipboard", target_os = "android"))]
pub use clipboard::AndroidClipboard;
#[cfg(all(
    feature = "clipboard",
    not(any(target_os = "android", target_os = "ios", target_env = "ohos"))
))]
pub use clipboard::OsClipboard;
#[cfg(feature = "clipboard")]
pub use clipboard::default_shared_clipboard;
pub use clipboard::{
    ClipboardHost, MemoryClipboard, SharedClipboardHost, UnsupportedClipboard,
    read_shared_clipboard, read_shared_clipboard_transfer, shared_clipboard,
    shared_clipboard_change_count, write_shared_clipboard, write_shared_clipboard_transfer,
};
#[cfg(feature = "fetch")]
pub use fetch::NativeFetchHost;
pub use fetch::{
    DEFAULT_FETCH_TIMEOUT, FetchCancellation, FetchError, FetchErrorKind, FetchHead, FetchHost,
    FetchPolicy, FetchRequest, FetchResponse, FetchSink, SharedFetchHost, shared_fetch_host,
};
pub use locale::{negotiate_language, preferred_languages};
pub use notification::{
    NotificationAction, NotificationBackendKind, NotificationCapability, NotificationDismissReason,
    NotificationError, NotificationEvent, NotificationOutcome, NotificationPermission,
    NotificationRequest, NotificationSupport, NotificationTimeout,
};
#[doc(hidden)]
pub use paths::__identity_marker_bytes;
pub use paths::{
    ApplicationIdentity, ApplicationLocation, ApplicationPaths, PathEnvironment, PathPlatform,
    PathsError, RuntimeLayout,
};
pub use persist::{
    FileStore, PersistedWindowGeometry, app_data_dir, persist_live_window_geometry,
    restore_window_geometry,
};
pub use window::{
    CONTENT_ZOOM_RANGE, DisplayBounds, DisplayId, DisplayInfo, FullscreenMode, FullscreenRequest,
    HitTestBehavior, InputRegionOutcome, InputRegionRect, InputRegionSnapshot,
    MousePassthroughMode, NativeHitTestClass, SurfacePresentationPolicy, SystemAppearance,
    WindowDescriptor, WindowEvent, WindowGeometry, WindowIcon, WindowIconError, WindowId,
    WindowInteractionCapabilities, WindowLevel, WindowModeState, WindowResizeEdge, WindowRole,
    WindowSurfacePreference, aspect_minimum_size, aspect_window_size, clamp_position_to_displays,
    clear_registered_application_icon, display_index_at, fit_window_to_displays,
    register_application_icon, resolve_window_icon, size_within_limits, valid_aspect_ratio,
    valid_content_zoom, window_resize_edge,
};
#[cfg(feature = "ws")]
pub use ws::{
    NativeWebSocketHost, SharedWebSocketHost, SocketPolicy, WebSocketHost, WsError, WsErrorKind,
    WsEvent, WsMessage, WsOpenRequest, WsSink, shared_websocket_host,
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
    /// Nana `DesktopShell` paint wired on this target.
    pub desktop_shell: bool,
    /// One Runtime document fills the Primary region below the system bars.
    pub primary_document: bool,
    /// Runtime controls (appearance page, Text + Input + Switch + Button) paint
    /// through the Scene painter.
    pub control_widget: bool,
    /// Host pointer events inside Primary route into the document.
    pub control_input: bool,
}

#[cfg(feature = "experimental-android")]
impl PlatformCapabilities {
    /// Android ARM64 host: Surface + Primary document controls/input.
    ///
    /// `ime` and `clipboard` are true once GameActivity InputConnection and
    /// JNI `ClipboardManager` are wired. This is not a second product paint
    /// kernel: the host still paints Runtime / UiScene through
    /// `SceneWgpuPainter`.
    pub const fn android_mvp() -> Self {
        Self {
            ime: true,
            clipboard: true,
            vulkan_surface: true,
            desktop_shell: false,
            primary_document: true,
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
        assert!(caps.ime);
        assert!(caps.clipboard);
        assert!(!caps.desktop_shell);
    }

    /// Shared layout fields via [`Default`] — Android does not fork `LayoutStyle`.
    #[test]
    fn experimental_android_shares_layout_fields() {
        let style = nana_ui_core::LayoutStyle::default();
        let _pointer_events: Option<nana_ui_core::PointerEventsSpec> = style.pointer_events;
        let _transform_3d: Option<nana_ui_core::PaintMat4> = style.transforms.transform_3d;
        let _logical_padding: nana_ui_core::LogicalEdges = style.logical_edges.padding;
        let _logical_margin: nana_ui_core::LogicalEdges = style.logical_edges.margin;
        let _logical_inset: nana_ui_core::LogicalEdges = style.logical_edges.inset;
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

/// Backend protocol for framework adapters. Applications use WindowService.
pub mod host {
    pub use crate::window::{MousePassthroughMode, WindowCommand};
}
/// The canonical input contract, owned by `nana-ui-input`.
pub use nana_ui_input::{
    CanonicalInputEvent, CommittedText, CompositionInput, CursorIcon, DEFAULT_MAX_TRANSFER_BYTES,
    DataFormat, DataFormatError, DataTransfer, DeviceId, DragCancelReason, DragSessionId,
    DropAccepts, DropEffect, DropEffects, DropKind, EndpointGeneration, FileDragInput,
    FileDragKind, FocusDirection, HeadlessHostServices, HostServiceError, HostServices,
    InputDisposition, InputEndpoint, InputMetadata, InputModifiers, InputPayload, InputSequence,
    InputSequencer, InputSourceId, InputTimestamp, InternalPayloadToken, KeyInput, KeyState,
    LazyFnProvider, LogicalKey, MAX_MIME_LEN, NavigationInput, NavigationIntent, NavigationSource,
    PhysicalKey, PointerId, PointerInput, PointerPhase, PointerType, SurroundingText,
    TextInputContext, TextInputPurpose, TransferAction, TransferCachePolicy, TransferEntry,
    TransferEntryMeta, TransferError, TransferImage, TransferLifetime, TransferOrigin,
    TransferProvider, UnsupportedHostServices, WheelInput, WheelUnit, file_uri_to_path,
    format_uri_list, gamepad_dpad_navigation, parse_uri_list, path_to_file_uri,
};
