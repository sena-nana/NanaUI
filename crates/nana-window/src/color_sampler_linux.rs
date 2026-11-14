use std::collections::HashMap;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_lite::future;
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

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn support() -> ColorSamplerSupport {
    let has_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let has_x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
    if has_wayland || has_x11 {
        ColorSamplerSupport::Native
    } else {
        ColorSamplerSupport::Unsupported
    }
}

fn parent_window_identifier(
    window: Option<&dyn HasWindowHandle>,
) -> Result<String, ColorSamplerError> {
    let Some(window) = window else {
        return Ok(String::new());
    };
    let handle = window
        .window_handle()
        .map_err(|_| ColorSamplerError::InvalidWindow)?;
    match handle.as_raw() {
        RawWindowHandle::Xlib(xlib) => Ok(format!("x11:{:#x}", xlib.window)),
        RawWindowHandle::Xcb(xcb) => Ok(format!("x11:{:#x}", xcb.window.get())),
        _ => Ok(String::new()),
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

enum PortalAttempt {
    Done(ColorSamplerResult),
    Unavailable(ColorSamplerError),
}

async fn pick_via_portal(
    request_id: u64,
    parent: &str,
    cancel_rx: &async_channel::Receiver<()>,
) -> PortalAttempt {
    let connection = match zbus::Connection::session().await {
        Ok(conn) => conn,
        Err(error) => return PortalAttempt::Unavailable(platform(error)),
    };
    let proxy = match Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
    )
    .await
    {
        Ok(proxy) => proxy,
        Err(error) => return PortalAttempt::Unavailable(platform(error)),
    };

    let sender = connection
        .unique_name()
        .map(|name| name.as_str().trim_start_matches(':').replace('.', "_"))
        .unwrap_or_else(|| "nana".into());
    let token = format!(
        "nana_color_{request_id}_{}",
        TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let expected_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");

    let rule = match MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .and_then(|b| b.member("Response"))
        .and_then(|b| b.path(expected_path.as_str()))
        .map(|b| b.build())
    {
        Ok(rule) => rule,
        Err(error) => return PortalAttempt::Unavailable(platform(error)),
    };

    let mut stream = match MessageStream::for_match_rule(rule, &connection, Some(4)).await {
        Ok(stream) => stream,
        Err(error) => return PortalAttempt::Unavailable(platform(error)),
    };

    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    let request_path: zbus::zvariant::OwnedObjectPath =
        match proxy.call("PickColor", &(parent, options)).await {
            Ok(path) => path,
            Err(error) => return PortalAttempt::Unavailable(platform(error)),
        };

    enum Outcome {
        Cancelled,
        Signal(Option<Result<zbus::Message, zbus::Error>>),
    }

    let next_signal = future::poll_fn(|cx| {
        use futures_lite::Stream;
        std::pin::Pin::new(&mut stream).poll_next(cx)
    });

    let outcome = future::or(
        async {
            let _ = cancel_rx.recv().await;
            Outcome::Cancelled
        },
        async { Outcome::Signal(next_signal.await) },
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
            Ok((status, values)) => {
                PortalAttempt::Done(portal_color_response(request_id, status, values))
            }
            Err(error) => {
                PortalAttempt::Done(ColorSamplerResult::failed(request_id, platform(error)))
            }
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

fn sample_x11_pixel(
    xlib: &Xlib,
    display: *mut Display,
    root: XWindow,
    x: i32,
    y: i32,
) -> Option<[f32; 3]> {
    // SAFETY: Queries a 1x1 pixel from `root` and resolves it against the default colormap.
    unsafe {
        let image = (xlib.XGetImage)(display, root, x, y, 1, 1, !0, ZPixmap);
        if image.is_null() {
            return None;
        }
        let pixel = (xlib.XGetPixel)(image, 0, 0);
        (xlib.XDestroyImage)(image);
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
                match event.get_type() {
                    ButtonPress => {
                        let btn = event.button;
                        if btn.button == Button1 {
                            return match sample_x11_pixel(
                                &xlib, display, root, btn.x_root, btn.y_root,
                            ) {
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
                    }
                    KeyPress => {
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
                    _ => {}
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
    let parent = parent_window_identifier(window)?;
    let (cancel_tx, cancel_rx) = async_channel::bounded::<()>(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_thread = Arc::clone(&cancelled);
    let cancelled_handle = Arc::clone(&cancelled);

    std::thread::Builder::new()
        .name("nana-color-sampler".into())
        .spawn(move || {
            let result = match future::block_on(pick_via_portal(request_id, &parent, &cancel_rx)) {
                PortalAttempt::Done(res) => res,
                PortalAttempt::Unavailable(portal_err) => {
                    let has_x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
                    if has_x11 && !cancelled_thread.load(Ordering::Acquire) {
                        pick_via_x11(request_id, &cancelled_thread)
                    } else if cancelled_thread.load(Ordering::Acquire) {
                        ColorSamplerResult::cancelled(request_id)
                    } else {
                        ColorSamplerResult::failed(request_id, portal_err)
                    }
                }
            };
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
}
