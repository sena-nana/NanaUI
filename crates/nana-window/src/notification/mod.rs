//! Native desktop notification host across macOS, Windows, and Linux.
//!
//! Backends:
//! - macOS: `UNUserNotificationCenter` (only when the process has a valid
//!   bundle identifier; unbundled tools explicitly report `Unavailable`).
//! - Windows: `Windows.UI.Notifications.ToastNotificationManager`.
//! - Linux: `org.freedesktop.Notifications` over session D-Bus.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub use nana_ui_platform::{
    NotificationAction, NotificationBackendKind, NotificationCapability, NotificationDismissReason,
    NotificationError, NotificationEvent, NotificationOutcome, NotificationPermission,
    NotificationRequest, NotificationSupport, NotificationTimeout,
};

type Wake = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct EventHub {
    wake: Mutex<Option<Wake>>,
    events: Mutex<VecDeque<NotificationEvent>>,
}

impl EventHub {
    fn new() -> Self {
        Self {
            wake: Mutex::new(None),
            events: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn push(&self, event: NotificationEvent) {
        if let Ok(mut queue) = self.events.lock() {
            queue.push_back(event);
        }
        let wake = self.wake.lock().ok().and_then(|guard| guard.clone());
        if let Some(wake) = wake {
            wake();
        }
    }

    fn take_all(&self) -> Vec<NotificationEvent> {
        let Ok(mut queue) = self.events.lock() else {
            return Vec::new();
        };
        queue.drain(..).collect()
    }

    fn set_wake(&self, wake: Option<Wake>) {
        if let Ok(mut slot) = self.wake.lock() {
            *slot = wake;
        }
    }
}

struct NotificationHostInner {
    hub: Arc<EventHub>,
    #[cfg(target_os = "macos")]
    backend: macos::MacNotificationBackend,
    #[cfg(target_os = "windows")]
    backend: windows::WindowsNotificationBackend,
    #[cfg(target_os = "linux")]
    backend: linux::LinuxNotificationBackend,
}

/// Thread-safe handle to the host desktop notification service.
#[derive(Clone)]
pub struct NotificationHost {
    inner: Arc<NotificationHostInner>,
}

impl Default for NotificationHost {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationHost {
    /// Creates a host handle bound to the current platform notification backend.
    pub fn new() -> Self {
        let hub = Arc::new(EventHub::new());
        Self {
            inner: Arc::new(NotificationHostInner {
                #[cfg(target_os = "macos")]
                backend: macos::MacNotificationBackend::new(Arc::clone(&hub)),
                #[cfg(target_os = "windows")]
                backend: windows::WindowsNotificationBackend::new(Arc::clone(&hub)),
                #[cfg(target_os = "linux")]
                backend: linux::LinuxNotificationBackend::new(Arc::clone(&hub)),
                hub,
            }),
        }
    }

    /// Registers a wake callback invoked whenever a notification interaction or
    /// dismissal is queued for [`Self::take_events`].
    pub fn set_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.inner.hub.set_wake(Some(Arc::new(wake)));
    }

    /// Queries backend availability, capabilities, and current authorization.
    pub fn support(&self) -> NotificationSupport {
        #[cfg(target_os = "macos")]
        {
            self.inner.backend.support()
        }
        #[cfg(target_os = "windows")]
        {
            self.inner.backend.support()
        }
        #[cfg(target_os = "linux")]
        {
            self.inner.backend.support()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            NotificationSupport {
                backend: NotificationBackendKind::Unavailable,
                permission: NotificationPermission::Unsupported,
                supports_actions: false,
                supports_replace: false,
                supports_images: false,
                unavailable_reason: Some(
                    "desktop notifications are unsupported on this target".into(),
                ),
            }
        }
    }

    /// Requests user authorization to display notifications where prompted by
    /// the platform (macOS `UNUserNotificationCenter`).
    pub fn request_permission(&self) -> Result<NotificationPermission, NotificationError> {
        #[cfg(target_os = "macos")]
        {
            self.inner.backend.request_permission()
        }
        #[cfg(target_os = "windows")]
        {
            self.inner.backend.request_permission()
        }
        #[cfg(target_os = "linux")]
        {
            self.inner.backend.request_permission()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Err(NotificationError::Unavailable(
                "desktop notifications are unsupported on this target".into(),
            ))
        }
    }

    /// Presents a new notification (or replaces an existing one sharing `request.tag`).
    pub fn present(
        &self,
        request: NotificationRequest,
    ) -> Result<NotificationOutcome, NotificationError> {
        validate_request(&request)?;
        #[cfg(target_os = "macos")]
        {
            self.inner.backend.present(request)
        }
        #[cfg(target_os = "windows")]
        {
            self.inner.backend.present(request)
        }
        #[cfg(target_os = "linux")]
        {
            self.inner.backend.present(request)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = request;
            Err(NotificationError::Unavailable(
                "desktop notifications are unsupported on this target".into(),
            ))
        }
    }

    /// Updates / replaces an already presented notification identified by `id`.
    pub fn update(
        &self,
        id: &str,
        request: NotificationRequest,
    ) -> Result<NotificationOutcome, NotificationError> {
        if id.trim().is_empty() {
            return Err(NotificationError::InvalidRequest(
                "notification id cannot be empty".into(),
            ));
        }
        validate_request(&request)?;
        #[cfg(target_os = "macos")]
        {
            self.inner.backend.update(id, request)
        }
        #[cfg(target_os = "windows")]
        {
            self.inner.backend.update(id, request)
        }
        #[cfg(target_os = "linux")]
        {
            self.inner.backend.update(id, request)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = (id, request);
            Err(NotificationError::Unavailable(
                "desktop notifications are unsupported on this target".into(),
            ))
        }
    }

    /// Withdraws a delivered or pending notification by `id`.
    pub fn withdraw(&self, id: &str) -> Result<(), NotificationError> {
        if id.trim().is_empty() {
            return Err(NotificationError::InvalidRequest(
                "notification id cannot be empty".into(),
            ));
        }
        #[cfg(target_os = "macos")]
        {
            self.inner.backend.withdraw(id)
        }
        #[cfg(target_os = "windows")]
        {
            self.inner.backend.withdraw(id)
        }
        #[cfg(target_os = "linux")]
        {
            self.inner.backend.withdraw(id)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = id;
            Err(NotificationError::Unavailable(
                "desktop notifications are unsupported on this target".into(),
            ))
        }
    }

    /// Withdraws all notifications managed by this host.
    pub fn clear(&self) -> Result<(), NotificationError> {
        #[cfg(target_os = "macos")]
        {
            self.inner.backend.clear()
        }
        #[cfg(target_os = "windows")]
        {
            self.inner.backend.clear()
        }
        #[cfg(target_os = "linux")]
        {
            self.inner.backend.clear()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Err(NotificationError::Unavailable(
                "desktop notifications are unsupported on this target".into(),
            ))
        }
    }

    /// Drains all queued activation / dismissal events in FIFO order.
    pub fn take_events(&self) -> Vec<NotificationEvent> {
        self.inner.hub.take_all()
    }
}

const MAX_NOTIFICATION_IMAGE_BYTES: usize = 4 << 20;
const MAX_NOTIFICATION_IMAGE_EDGE: u32 = 1024;

pub(crate) fn validate_request(request: &NotificationRequest) -> Result<(), NotificationError> {
    if request.title.trim().is_empty() && request.body.trim().is_empty() {
        return Err(NotificationError::InvalidRequest(
            "notification title and body cannot both be empty".into(),
        ));
    }
    if let Some(tag) = request.tag.as_deref()
        && tag.trim().is_empty()
    {
        return Err(NotificationError::InvalidRequest(
            "notification tag cannot be blank when specified".into(),
        ));
    }
    for action in &request.actions {
        if action.id.trim().is_empty() || action.label.trim().is_empty() {
            return Err(NotificationError::InvalidRequest(
                "notification actions require non-empty id and label".into(),
            ));
        }
    }
    if let Some(bytes) = request.image_png.as_deref() {
        if bytes.is_empty() || bytes.len() > MAX_NOTIFICATION_IMAGE_BYTES {
            return Err(NotificationError::InvalidRequest(
                "notification PNG image exceeds size bounds".into(),
            ));
        }
    }
    Ok(())
}

/// Escapes special XML characters for Windows Toast XML payloads.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Builds a `ToastNotification` XML template string from `request`.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn build_windows_toast_xml(
    notification_id: &str,
    request: &NotificationRequest,
    image_src_uri: Option<&str>,
) -> String {
    let launch_arg = format!("nana_id={}", escape_xml(notification_id));
    let mut xml = format!(r#"<toast launch="{launch_arg}">"#);
    xml.push_str(r#"<visual><binding template="ToastGeneric">"#);
    if !request.title.is_empty() {
        xml.push_str("<text>");
        xml.push_str(&escape_xml(&request.title));
        xml.push_str("</text>");
    }
    if !request.body.is_empty() {
        xml.push_str("<text>");
        xml.push_str(&escape_xml(&request.body));
        xml.push_str("</text>");
    }
    if let Some(uri) = image_src_uri {
        xml.push_str(r#"<image placement="hero" src=""#);
        xml.push_str(&escape_xml(uri));
        xml.push_str(r#""/>"#);
    }
    xml.push_str("</binding></visual>");
    if !request.actions.is_empty() {
        xml.push_str("<actions>");
        for action in &request.actions {
            let args = format!(
                "nana_id={}&amp;action={}",
                escape_xml(notification_id),
                escape_xml(&action.id)
            );
            xml.push_str(&format!(
                r#"<action content="{}" arguments="{}"/>"#,
                escape_xml(&action.label),
                args
            ));
        }
        xml.push_str("</actions>");
    }
    if request.silent {
        xml.push_str(r#"<audio silent="true"/>"#);
    }
    xml.push_str("</toast>");
    xml
}

/// Decodes a PNG image into freedesktop `image-data` tuple fields:
/// `(width, height, rowstride, has_alpha, bits_per_sample, channels, rgba)`.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn decode_png_freedesktop_image_data(
    bytes: &[u8],
) -> Result<(i32, i32, i32, bool, i32, i32, Vec<u8>), NotificationError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    decoder.set_limits(png::Limits {
        bytes: MAX_NOTIFICATION_IMAGE_BYTES,
    });
    let mut reader = decoder
        .read_info()
        .map_err(|e| NotificationError::InvalidRequest(format!("invalid notification PNG: {e}")))?;
    let buf_size = reader
        .output_buffer_size()
        .ok_or_else(|| NotificationError::InvalidRequest("invalid PNG buffer size".into()))?;
    let mut raw = vec![0; buf_size];
    let frame = reader
        .next_frame(&mut raw)
        .map_err(|e| NotificationError::InvalidRequest(format!("invalid notification PNG: {e}")))?;
    if frame.width == 0
        || frame.height == 0
        || frame.width > MAX_NOTIFICATION_IMAGE_EDGE
        || frame.height > MAX_NOTIFICATION_IMAGE_EDGE
    {
        return Err(NotificationError::InvalidRequest(
            "notification image dimensions out of bounds".into(),
        ));
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => {
            return Err(NotificationError::InvalidRequest(
                "indexed notification PNG unsupported".into(),
            ));
        }
    };
    let mut rgba = Vec::with_capacity(width * height * 4);
    for px in raw[..width * height * channels].chunks_exact(channels) {
        let (r, g, b, a) = match *px {
            [r, g, b, a] => (r, g, b, a),
            [r, g, b] => (r, g, b, 255),
            [l, a] => (l, l, l, a),
            [l] => (l, l, l, 255),
            _ => unreachable!(),
        };
        rgba.extend_from_slice(&[r, g, b, a]);
    }
    let w = frame.width as i32;
    let h = frame.height as i32;
    let stride = w * 4;
    Ok((w, h, stride, true, 8, 4, rgba))
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn timeout_to_freedesktop_ms(timeout: NotificationTimeout) -> i32 {
    match timeout {
        NotificationTimeout::Default => -1,
        NotificationTimeout::Persistent => 0,
        NotificationTimeout::Millis(ms) => i32::try_from(ms).unwrap_or(i32::MAX).max(1),
    }
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn parse_windows_activation_args(args: &str) -> (Option<String>, Option<String>) {
    let mut id = None;
    let mut action = None;
    for pair in args.split('&') {
        if let Some((key, val)) = pair.split_once('=') {
            match key {
                "nana_id" if !val.is_empty() => id = Some(val.to_owned()),
                "action" if !val.is_empty() => action = Some(val.to_owned()),
                _ => {}
            }
        }
    }
    (id, action)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn validates_empty_and_malformed_requests() {
        let empty = NotificationRequest::new("   ", "");
        assert!(matches!(
            validate_request(&empty),
            Err(NotificationError::InvalidRequest(_))
        ));

        let blank_tag = NotificationRequest::new("Title", "Body").tag("   ");
        assert!(matches!(
            validate_request(&blank_tag),
            Err(NotificationError::InvalidRequest(_))
        ));

        let bad_action = NotificationRequest::new("Title", "Body").action("", "Open");
        assert!(matches!(
            validate_request(&bad_action),
            Err(NotificationError::InvalidRequest(_))
        ));

        let valid = NotificationRequest::new("Build finished", "All checks passed")
            .tag("build")
            .action("open", "Open");
        assert!(validate_request(&valid).is_ok());
    }

    #[test]
    fn windows_toast_xml_escapes_special_characters_and_parses_args() {
        let req = NotificationRequest::new("Hello <User> & \"Team\"", "100% 'done'")
            .action("reply&go", "Reply <Now>")
            .silent(true);
        let xml = build_windows_toast_xml("id-1", &req, Some("file:///tmp/a&b.png"));
        assert!(xml.contains("&lt;User&gt; &amp; &quot;Team&quot;"));
        assert!(xml.contains("100% &apos;done&apos;"));
        assert!(xml.contains(r#"<audio silent="true"/>"#));
        assert!(xml.contains(r#"arguments="nana_id=id-1&amp;action=reply&amp;go""#));

        assert_eq!(
            parse_windows_activation_args("nana_id=n-9&action=open"),
            (Some("n-9".into()), Some("open".into()))
        );
        assert_eq!(
            parse_windows_activation_args("nana_id=n-9"),
            (Some("n-9".into()), None)
        );
    }

    #[test]
    fn freedesktop_image_data_and_timeout_map_cleanly() {
        let png_bytes = crate::tray::encode_rgba_png(2, 1, &[1, 2, 3, 4, 5, 6, 7, 255]).unwrap();
        let (w, h, stride, alpha, bps, ch, rgba) =
            decode_png_freedesktop_image_data(&png_bytes).unwrap();
        assert_eq!((w, h, stride, alpha, bps, ch), (2, 1, 8, true, 8, 4));
        assert_eq!(rgba, vec![1, 2, 3, 4, 5, 6, 7, 255]);

        assert_eq!(timeout_to_freedesktop_ms(NotificationTimeout::Default), -1);
        assert_eq!(
            timeout_to_freedesktop_ms(NotificationTimeout::Persistent),
            0
        );
        assert_eq!(
            timeout_to_freedesktop_ms(NotificationTimeout::Millis(2500)),
            2500
        );
    }

    #[test]
    fn event_hub_queues_and_fires_wake_in_order() {
        let host = NotificationHost::new();
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_clone = Arc::clone(&wakes);
        host.set_wake(move || {
            wakes_clone.fetch_add(1, Ordering::SeqCst);
        });
        host.inner.hub.push(NotificationEvent::Activated {
            id: "n1".into(),
            action_id: Some("open".into()),
        });
        host.inner.hub.push(NotificationEvent::Dismissed {
            id: "n2".into(),
            reason: NotificationDismissReason::User,
        });
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        let events = host.take_events();
        assert_eq!(events.len(), 2);
        assert!(host.take_events().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unbundled_macos_test_binary_reports_unavailable_without_crashing() {
        let host = NotificationHost::new();
        let support = host.support();
        assert_eq!(support.backend, NotificationBackendKind::Unavailable);
        assert_eq!(support.permission, NotificationPermission::Unsupported);
        assert!(!support.is_available());
        assert!(support.unavailable_reason.is_some());
        assert!(matches!(
            host.present(NotificationRequest::new("Hello", "World")),
            Err(NotificationError::Unavailable(_))
        ));
    }
}
