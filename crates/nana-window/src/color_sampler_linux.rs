use std::collections::HashMap;
use std::ffi::OsString;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_lite::{StreamExt, future};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11_dl::keysym::{XK_Escape, XK_Return, XK_space};
use x11_dl::xlib::{
    Button1, Button3, ButtonPress, ButtonPressMask, ButtonReleaseMask, CurrentTime, Display,
    GrabModeAsync, GrabSuccess, KeyPress, PointerMotionMask, Window as XWindow, XColor, XEvent,
    Xlib, ZPixmap,
};
use zbus::MatchRule;
use zbus::MessageStream;
use zbus::Proxy;
use zbus::zvariant::{OwnedValue, Value};

use crate::color_sampler::{
    ColorSamplerError, ColorSamplerHandle, ColorSamplerResult, ColorSamplerSupport, srgb_u16_to_rgb,
};

const XC_CROSSHAIR: u32 = 34;
const PORTAL_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

type PortalCacheKey = (Option<OsString>, Option<OsString>, Option<OsString>);

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);
static PORTAL_SUPPORT_CACHE: Mutex<Option<(PortalCacheKey, bool)>> = Mutex::new(None);

async fn probe_portal_pick_color() -> bool {
    let Ok(connection) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(proxy) = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
    )
    .await
    else {
        return false;
    };
    matches!(proxy.get_property::<u32>("version").await, Ok(version) if version >= 2)
}

fn portal_pick_color_supported() -> bool {
    let key = (
        std::env::var_os("WAYLAND_DISPLAY"),
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS"),
        std::env::var_os("XDG_RUNTIME_DIR"),
    );
    if let Ok(guard) = PORTAL_SUPPORT_CACHE.lock()
        && let Some((cached_key, cached)) = guard.as_ref()
        && *cached_key == key
        && *cached
    {
        return true;
    }

    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("nana-portal-probe".into())
        .spawn(move || {
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                future::block_on(probe_portal_pick_color())
            }))
            .unwrap_or(false);
            let _ = tx.send(ok);
        });
    let supported = if spawned.is_ok() {
        match rx.recv_timeout(PORTAL_PROBE_TIMEOUT) {
            Ok(supported) => supported,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Don't cache timeout as false, just return false for now
                false
            }
            Err(_) => false,
        }
    } else {
        false
    };
    if supported && let Ok(mut guard) = PORTAL_SUPPORT_CACHE.lock() {
        *guard = Some((key, true));
    }
    supported
}

fn is_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
        || std::env::var_os("XDG_SESSION_TYPE")
            .and_then(|v| v.into_string().ok())
            .is_some_and(|s| s.eq_ignore_ascii_case("wayland"))
}

fn allow_x11_fallback(is_wayland: bool, has_x11_display: bool) -> bool {
    !is_wayland && has_x11_display
}

fn support_for(
    has_wayland: bool,
    has_x11: bool,
    probe_portal: impl FnOnce() -> bool,
) -> ColorSamplerSupport {
    if has_wayland {
        return if probe_portal() {
            ColorSamplerSupport::Native
        } else {
            ColorSamplerSupport::Unsupported
        };
    }
    if has_x11 {
        return ColorSamplerSupport::Native;
    }
    ColorSamplerSupport::Unsupported
}

pub(crate) fn support() -> ColorSamplerSupport {
    let has_wayland = is_wayland_session();
    let has_x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
    support_for(has_wayland, has_x11, portal_pick_color_supported)
}

fn parent_window_identifier(
    window: Option<&dyn HasWindowHandle>,
) -> Result<(String, bool), ColorSamplerError> {
    let Some(window) = window else {
        return Ok((String::new(), false));
    };
    let handle = window
        .window_handle()
        .map_err(|_| ColorSamplerError::InvalidWindow)?;
    match handle.as_raw() {
        RawWindowHandle::Xlib(xlib) => Ok((format!("x11:{:#x}", xlib.window), false)),
        RawWindowHandle::Xcb(xcb) => Ok((format!("x11:{:#x}", xcb.window.get()), false)),
        RawWindowHandle::Wayland(_) => Ok((String::new(), true)),
        _ => Ok((String::new(), false)),
    }
}

fn platform(error: impl std::fmt::Display) -> ColorSamplerError {
    ColorSamplerError::Platform(error.to_string())
}

fn portal_color_response(
    request_id: u64,
    status: u32,
    mut values: HashMap<String, OwnedValue>,
) -> ColorSamplerResult {
    if status == 1 {
        return ColorSamplerResult::cancelled(request_id);
    }
    if status != 0 {
        return ColorSamplerResult::failed(
            request_id,
            platform(format!("portal rejected color selection ({status})")),
        );
    }
    let Some(raw) = values.remove("color") else {
        return ColorSamplerResult::failed(request_id, platform("portal omitted selected color"));
    };
    match <(f64, f64, f64)>::try_from(raw) {
        Ok((r, g, b)) => ColorSamplerResult::picked(request_id, [r as f32, g as f32, b as f32]),
        Err(error) => ColorSamplerResult::failed(request_id, platform(error)),
    }
}

#[derive(Debug, PartialEq)]
enum PortalAttempt {
    Done(ColorSamplerResult),
    Unavailable(ColorSamplerError),
}

fn classify_portal_error(error: &zbus::Error) -> ColorSamplerError {
    match error {
        zbus::Error::InterfaceNotFound
        | zbus::Error::Unsupported
        | zbus::Error::Address(_)
        | zbus::Error::Handshake(_)
        | zbus::Error::Connection(_, _) => ColorSamplerError::Unsupported,
        zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut => {
            ColorSamplerError::Platform(format!("portal probe timed out: {io}"))
        }
        zbus::Error::MethodError(name, _, _)
            if matches!(
                name.as_str(),
                "org.freedesktop.DBus.Error.ServiceUnknown"
                    | "org.freedesktop.DBus.Error.NameHasNoOwner"
                    | "org.freedesktop.DBus.Error.UnknownMethod"
                    | "org.freedesktop.DBus.Error.UnknownInterface"
                    | "org.freedesktop.DBus.Error.UnknownProperty"
                    | "org.freedesktop.DBus.Error.UnknownObject"
                    | "org.freedesktop.DBus.Error.NotSupported"
            ) =>
        {
            ColorSamplerError::Unsupported
        }
        zbus::Error::FDO(fdo)
            if matches!(
                fdo.as_ref(),
                zbus::fdo::Error::ServiceUnknown(_)
                    | zbus::fdo::Error::NameHasNoOwner(_)
                    | zbus::fdo::Error::UnknownMethod(_)
                    | zbus::fdo::Error::UnknownInterface(_)
                    | zbus::fdo::Error::UnknownProperty(_)
                    | zbus::fdo::Error::UnknownObject(_)
                    | zbus::fdo::Error::NotSupported(_)
            ) =>
        {
            ColorSamplerError::Unsupported
        }
        _ => platform(error),
    }
}

fn portal_response_attempt(
    request_id: u64,
    status: u32,
    values: HashMap<String, OwnedValue>,
) -> PortalAttempt {
    match portal_color_response(request_id, status, values) {
        ColorSamplerResult::Failed { error, .. } => PortalAttempt::Unavailable(error),
        other => PortalAttempt::Done(other),
    }
}

async fn until_cancel<F: std::future::Future>(
    work: F,
    cancel_rx: &async_channel::Receiver<()>,
    cancelled: &AtomicBool,
) -> Option<F::Output> {
    if cancelled.load(Ordering::Acquire) {
        return None;
    }
    future::race(async { Some(work.await) }, async {
        let _ = cancel_rx.recv().await;
        None
    })
    .await
}

async fn pick_via_portal(
    request_id: u64,
    parent: &str,
    cancel_rx: &async_channel::Receiver<()>,
    cancelled: &AtomicBool,
) -> PortalAttempt {
    let Some(connection_res) =
        until_cancel(zbus::Connection::session(), cancel_rx, cancelled).await
    else {
        return PortalAttempt::Done(ColorSamplerResult::cancelled(request_id));
    };
    let connection = match connection_res {
        Ok(conn) => conn,
        Err(error) => return PortalAttempt::Unavailable(classify_portal_error(&error)),
    };

    let Some(proxy_res) = until_cancel(
        Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Screenshot",
        ),
        cancel_rx,
        cancelled,
    )
    .await
    else {
        return PortalAttempt::Done(ColorSamplerResult::cancelled(request_id));
    };
    let proxy = match proxy_res {
        Ok(proxy) => proxy,
        Err(error) => return PortalAttempt::Unavailable(classify_portal_error(&error)),
    };

    let Some(version_res) =
        until_cancel(proxy.get_property::<u32>("version"), cancel_rx, cancelled).await
    else {
        return PortalAttempt::Done(ColorSamplerResult::cancelled(request_id));
    };
    match version_res {
        Ok(version) if version >= 2 => {}
        Ok(_) => return PortalAttempt::Unavailable(ColorSamplerError::Unsupported),
        Err(error) => return PortalAttempt::Unavailable(classify_portal_error(&error)),
    }

    let token = format!(
        "nana_color_{request_id}_{}",
        TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
    );

    // Subscribe before PickColor without a predicted object path: portals may
    // ignore handle_token or emit an early Response before PickColor returns.
    let rule = match MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.portal.Desktop")
        .and_then(|b| b.interface("org.freedesktop.portal.Request"))
        .and_then(|b| b.member("Response"))
        .map(|b| b.build())
    {
        Ok(rule) => rule,
        Err(error) => return PortalAttempt::Unavailable(platform(error)),
    };

    let Some(stream_res) = until_cancel(
        MessageStream::for_match_rule(rule, &connection, Some(8)),
        cancel_rx,
        cancelled,
    )
    .await
    else {
        return PortalAttempt::Done(ColorSamplerResult::cancelled(request_id));
    };
    let mut stream = match stream_res {
        Ok(stream) => stream,
        Err(error) => return PortalAttempt::Unavailable(classify_portal_error(&error)),
    };

    if cancelled.load(Ordering::Acquire) {
        return PortalAttempt::Done(ColorSamplerResult::cancelled(request_id));
    }

    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    let body = (parent, options);
    let pick_call = proxy.call::<_, _, zbus::zvariant::OwnedObjectPath>("PickColor", &body);
    let Some(request_res) = until_cancel(pick_call, cancel_rx, cancelled).await else {
        let _ = connection.clone().close().await;
        return PortalAttempt::Done(ColorSamplerResult::cancelled(request_id));
    };
    let request_path = match request_res {
        Ok(path) => path,
        Err(error) => return PortalAttempt::Unavailable(classify_portal_error(&error)),
    };

    enum Outcome {
        Cancelled,
        Signal(Option<Result<zbus::Message, zbus::Error>>),
    }

    let outcome = future::race(
        async {
            loop {
                match stream.next().await {
                    Some(Ok(message))
                        if message.header().path().map(|path| path.as_str())
                            == Some(request_path.as_str()) =>
                    {
                        break Outcome::Signal(Some(Ok(message)));
                    }
                    Some(Ok(_)) => continue,
                    other => break Outcome::Signal(other),
                }
            }
        },
        async {
            if !cancelled.load(Ordering::Acquire) {
                let _ = cancel_rx.recv().await;
            }
            Outcome::Cancelled
        },
    )
    .await;

    match outcome {
        Outcome::Cancelled => {
            if let Ok(req_proxy) = Proxy::new(
                &connection,
                "org.freedesktop.portal.Desktop",
                request_path.as_str(),
                "org.freedesktop.portal.Request",
            )
            .await
            {
                let _: Result<(), _> = req_proxy.call("Close", &()).await;
            }
            PortalAttempt::Done(ColorSamplerResult::cancelled(request_id))
        }
        Outcome::Signal(Some(Ok(message))) => match message
            .body()
            .deserialize::<(u32, HashMap<String, OwnedValue>)>()
        {
            Ok((status, values)) => portal_response_attempt(request_id, status, values),
            Err(error) => PortalAttempt::Unavailable(platform(error)),
        },
        Outcome::Signal(Some(Err(error))) => {
            PortalAttempt::Done(ColorSamplerResult::failed(request_id, platform(error)))
        }
        Outcome::Signal(None) => PortalAttempt::Done(ColorSamplerResult::failed(
            request_id,
            platform("portal response stream closed unexpectedly"),
        )),
    }
}

struct X11GrabGuard<'a> {
    xlib: &'a Xlib,
    display: *mut Display,
    cursor: u64,
    pointer_grabbed: bool,
    keyboard_grabbed: bool,
}

impl Drop for X11GrabGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: Releases X11 pointer/keyboard grabs, temporary cursor, and display opened on this thread.
        unsafe {
            if self.pointer_grabbed {
                (self.xlib.XUngrabPointer)(self.display, CurrentTime);
            }
            if self.keyboard_grabbed {
                (self.xlib.XUngrabKeyboard)(self.display, CurrentTime);
            }
            if self.cursor != 0 {
                (self.xlib.XFreeCursor)(self.display, self.cursor);
            }
            if !self.display.is_null() {
                (self.xlib.XSync)(self.display, 0);
                (self.xlib.XCloseDisplay)(self.display);
            }
        }
    }
}

fn decode_x11_channel(pixel: u64, mask: u64) -> Option<f32> {
    if mask == 0 {
        return None;
    }
    let shift = mask.trailing_zeros();
    let shifted_mask = mask >> shift;
    if (shifted_mask & shifted_mask.wrapping_add(1)) != 0 {
        return None;
    }
    let raw = (pixel & mask) >> shift;
    Some((raw as f32 / shifted_mask as f32).clamp(0.0, 1.0))
}

fn decode_x11_masked_rgb(pixel: u64, r_mask: u64, g_mask: u64, b_mask: u64) -> Option<[f32; 3]> {
    if r_mask == 0 || g_mask == 0 || b_mask == 0 {
        return None;
    }
    Some([
        decode_x11_channel(pixel, r_mask)?,
        decode_x11_channel(pixel, g_mask)?,
        decode_x11_channel(pixel, b_mask)?,
    ])
}

fn sample_x11_pixel(
    xlib: &Xlib,
    display: *mut Display,
    root: XWindow,
    x: i32,
    y: i32,
) -> Option<[f32; 3]> {
    // SAFETY: Queries a 1x1 pixel from `root` and decodes via visual masks before falling back to the default colormap.
    unsafe {
        let image = (xlib.XGetImage)(display, root, x, y, 1, 1, !0, ZPixmap);
        if image.is_null() {
            return None;
        }
        let pixel = (xlib.XGetPixel)(image, 0, 0);
        #[allow(clippy::useless_conversion)]
        let (r_mask, g_mask, b_mask, pixel_u64) = (
            u64::from((*image).red_mask),
            u64::from((*image).green_mask),
            u64::from((*image).blue_mask),
            u64::from(pixel),
        );
        (xlib.XDestroyImage)(image);
        if let Some(rgb) = decode_x11_masked_rgb(pixel_u64, r_mask, g_mask, b_mask) {
            return Some(rgb);
        }
        let screen = (xlib.XDefaultScreen)(display);
        let colormap = (xlib.XDefaultColormap)(display, screen);
        let mut color = XColor {
            pixel,
            red: 0,
            green: 0,
            blue: 0,
            flags: 0,
            pad: 0,
        };
        if (xlib.XQueryColor)(display, colormap, &mut color) == 0 {
            return None;
        }
        Some(srgb_u16_to_rgb(color.red, color.green, color.blue))
    }
}

fn pick_via_x11(request_id: u64, cancelled: &AtomicBool) -> ColorSamplerResult {
    let xlib = match Xlib::open() {
        Ok(xlib) => xlib,
        Err(error) => return ColorSamplerResult::failed(request_id, platform(error)),
    };
    // SAFETY: Opens a temporary X11 display connection, grabs root pointer & keyboard, and waits for click or Esc.
    unsafe {
        let display = (xlib.XOpenDisplay)(ptr::null());
        if display.is_null() {
            return ColorSamplerResult::failed(
                request_id,
                platform("could not open X11 display for color sampling"),
            );
        }
        let root = (xlib.XDefaultRootWindow)(display);
        let cursor = (xlib.XCreateFontCursor)(display, XC_CROSSHAIR);
        let mut guard = X11GrabGuard {
            xlib: &xlib,
            display,
            cursor,
            pointer_grabbed: false,
            keyboard_grabbed: false,
        };

        let ptr_status = (xlib.XGrabPointer)(
            display,
            root,
            0,
            (ButtonPressMask | ButtonReleaseMask | PointerMotionMask) as u32,
            GrabModeAsync,
            GrabModeAsync,
            0,
            cursor,
            CurrentTime,
        );
        if ptr_status != GrabSuccess {
            return ColorSamplerResult::failed(
                request_id,
                platform("could not grab X11 pointer for color sampling"),
            );
        }
        guard.pointer_grabbed = true;

        if (xlib.XGrabKeyboard)(display, root, 0, GrabModeAsync, GrabModeAsync, CurrentTime)
            == GrabSuccess
        {
            guard.keyboard_grabbed = true;
        }
        (xlib.XSync)(display, 0);

        loop {
            if cancelled.load(Ordering::Acquire) {
                return ColorSamplerResult::cancelled(request_id);
            }
            if (xlib.XPending)(display) > 0 {
                let mut event: XEvent = std::mem::zeroed();
                (xlib.XNextEvent)(display, &mut event);
                let event_type = event.get_type();
                if event_type == ButtonPress {
                    let btn = event.button;
                    if btn.button == Button1 {
                        return match sample_x11_pixel(&xlib, display, root, btn.x_root, btn.y_root)
                        {
                            Some(rgb) => ColorSamplerResult::picked(request_id, rgb),
                            None => ColorSamplerResult::failed(
                                request_id,
                                platform("could not read pixel from X11 root window"),
                            ),
                        };
                    }
                    if btn.button == Button3 {
                        return ColorSamplerResult::cancelled(request_id);
                    }
                } else if event_type == KeyPress {
                    let mut key = event.key;
                    let sym = (xlib.XLookupKeysym)(&mut key, 0) as u32;
                    if sym == XK_Escape {
                        return ColorSamplerResult::cancelled(request_id);
                    }
                    if sym == XK_Return || sym == XK_space {
                        let (mut q_root, mut q_child) = (0u64, 0u64);
                        let (mut rx, mut ry, mut wx, mut wy) = (0i32, 0i32, 0i32, 0i32);
                        let mut mask = 0u32;
                        if (xlib.XQueryPointer)(
                            display,
                            root,
                            &mut q_root,
                            &mut q_child,
                            &mut rx,
                            &mut ry,
                            &mut wx,
                            &mut wy,
                            &mut mask,
                        ) != 0
                        {
                            return match sample_x11_pixel(&xlib, display, root, rx, ry) {
                                Some(rgb) => ColorSamplerResult::picked(request_id, rgb),
                                None => ColorSamplerResult::failed(
                                    request_id,
                                    platform("could not read pixel from X11 root window"),
                                ),
                            };
                        }
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(8));
            }
        }
    }
}

pub(crate) fn pick_screen_color<F>(
    window: Option<&dyn HasWindowHandle>,
    request_id: u64,
    completion: F,
) -> Result<ColorSamplerHandle, ColorSamplerError>
where
    F: FnOnce(ColorSamplerResult) + Send + 'static,
{
    let (parent, window_is_wayland) = parent_window_identifier(window)?;
    let is_wayland = window_is_wayland || is_wayland_session();
    let (cancel_tx, cancel_rx) = async_channel::bounded::<()>(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_thread = Arc::clone(&cancelled);
    let cancelled_handle = Arc::clone(&cancelled);

    std::thread::Builder::new()
        .name("nana-color-sampler".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match future::block_on(pick_via_portal(
                    request_id,
                    &parent,
                    &cancel_rx,
                    &cancelled_thread,
                )) {
                    PortalAttempt::Done(res) => res,
                    PortalAttempt::Unavailable(portal_err) => {
                        let has_x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
                        if allow_x11_fallback(is_wayland, has_x11)
                            && !cancelled_thread.load(Ordering::Acquire)
                        {
                            pick_via_x11(request_id, &cancelled_thread)
                        } else if cancelled_thread.load(Ordering::Acquire) {
                            ColorSamplerResult::cancelled(request_id)
                        } else {
                            ColorSamplerResult::failed(request_id, portal_err)
                        }
                    }
                }
            }))
            .unwrap_or_else(|_| {
                ColorSamplerResult::failed(
                    request_id,
                    ColorSamplerError::Platform("color sampler backend panicked".into()),
                )
            });
            completion(result);
        })
        .map(|_| {
            ColorSamplerHandle::new(move || {
                cancelled_handle.store(true, Ordering::Release);
                let _ = cancel_tx.try_send(());
            })
        })
        .map_err(|error| ColorSamplerError::Platform(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_response_maps_picked_cancel_and_error() {
        let mut ok_values = HashMap::new();
        ok_values.insert(
            "color".to_owned(),
            OwnedValue::try_from(Value::from((0.25_f64, 0.5_f64, 0.75_f64))).unwrap(),
        );
        assert_eq!(
            portal_color_response(11, 0, ok_values),
            ColorSamplerResult::Picked {
                request_id: 11,
                rgba: [0.25, 0.5, 0.75, 1.0],
            }
        );
        assert!(portal_color_response(11, 1, HashMap::new()).is_cancelled());
        assert!(matches!(
            portal_color_response(11, 2, HashMap::new()),
            ColorSamplerResult::Failed { .. }
        ));
        assert!(matches!(
            portal_color_response(11, 0, HashMap::new()),
            ColorSamplerResult::Failed { .. }
        ));
    }

    #[test]
    fn portal_status_two_and_missing_color_yield_unavailable_for_x11_fallback() {
        assert!(matches!(
            portal_response_attempt(5, 2, HashMap::new()),
            PortalAttempt::Unavailable(_)
        ));
        assert!(matches!(
            portal_response_attempt(5, 0, HashMap::new()),
            PortalAttempt::Unavailable(_)
        ));
        assert!(matches!(
            portal_response_attempt(5, 1, HashMap::new()),
            PortalAttempt::Done(ColorSamplerResult::Cancelled { request_id: 5 })
        ));
    }

    #[test]
    fn wayland_without_capable_portal_reports_unsupported() {
        assert_eq!(
            support_for(true, false, || false),
            ColorSamplerSupport::Unsupported
        );
        assert_eq!(
            support_for(true, true, || false),
            ColorSamplerSupport::Unsupported
        );
        assert_eq!(
            support_for(true, true, || true),
            ColorSamplerSupport::Native
        );
        assert_eq!(
            support_for(true, false, || true),
            ColorSamplerSupport::Native
        );
        assert_eq!(
            support_for(false, true, || false),
            ColorSamplerSupport::Native
        );
        assert_eq!(
            support_for(false, false, || true),
            ColorSamplerSupport::Unsupported
        );
        assert!(!allow_x11_fallback(true, true));
        assert!(allow_x11_fallback(false, true));
        assert!(!allow_x11_fallback(false, false));
    }

    #[test]
    fn x11_masked_pixel_decoding_handles_bgrx_rgb565_deep_color_and_indexed_fallback() {
        // 24/32-bit TrueColor BGRX (0x00RRGGBB)
        assert_eq!(
            decode_x11_masked_rgb(0x00ff_8040, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff),
            Some([1.0, 128.0 / 255.0, 64.0 / 255.0])
        );
        // Swapped XBGR byte order (e.g. big-endian / BGR visual)
        assert_eq!(
            decode_x11_masked_rgb(0x0040_80ff, 0x0000_00ff, 0x0000_ff00, 0x00ff_0000),
            Some([1.0, 128.0 / 255.0, 64.0 / 255.0])
        );
        // 16-bit RGB565
        assert_eq!(
            decode_x11_masked_rgb(0xf800 | 0x07e0, 0xf800, 0x07e0, 0x001f),
            Some([1.0, 1.0, 0.0])
        );
        // 30-bit (10-bit per channel) TrueColor
        assert_eq!(
            decode_x11_masked_rgb(
                0x3ff0_0000 | 0x000f_fc00,
                0x3ff0_0000,
                0x000f_fc00,
                0x0000_03ff
            ),
            Some([1.0, 1.0, 0.0])
        );
        // Indexed / PseudoColor visual (zero masks) falls back to XQueryColor
        assert_eq!(decode_x11_masked_rgb(42, 0, 0, 0), None);
        // Non-contiguous channel mask falls back to XQueryColor
        assert_eq!(decode_x11_masked_rgb(0x55, 0x55, 0x00ff_0000, 0xff), None);
    }

    #[test]
    fn dbus_missing_service_or_interface_classifies_as_unsupported() {
        assert_eq!(
            classify_portal_error(&zbus::Error::InterfaceNotFound),
            ColorSamplerError::Unsupported
        );
        assert_eq!(
            classify_portal_error(&zbus::Error::Unsupported),
            ColorSamplerError::Unsupported
        );
        assert_eq!(
            classify_portal_error(&zbus::Error::FDO(Box::new(
                zbus::fdo::Error::UnknownProperty("version".into())
            ))),
            ColorSamplerError::Unsupported
        );
        assert!(matches!(
            classify_portal_error(&zbus::Error::Failure("other".into())),
            ColorSamplerError::Platform(_)
        ));
    }
}
