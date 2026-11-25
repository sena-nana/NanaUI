//! Linux `org.freedesktop.portal.Settings` reader and change listener for
//! `org.freedesktop.appearance` (`reduced-motion` and `high-contrast`).
//!
//! Queries use a bounded 500 ms D-Bus timeout so desktops without XDG Desktop
//! Portal fall back cleanly to `None`. Once initialized, a single background
//! daemon listens for `SettingChanged` signals, updates the cached booleans,
//! notifies `motion_preference` / `contrast_preference`, and invokes the
//! registered accessibility preference waker.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, Once};
use std::time::Duration;

use futures_lite::future;
use futures_lite::stream::StreamExt;
use zbus::zvariant::OwnedValue;

const PORTAL_TIMEOUT: Duration = Duration::from_millis(500);
const PORTAL_SERVICE: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS_IFACE: &str = "org.freedesktop.portal.Settings";
const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";
const KEY_REDUCED_MOTION: &str = "reduced-motion";
const KEY_HIGH_CONTRAST: &str = "high-contrast";

/// `0` = unknown/unset, `1` = false, `2` = true.
static REDUCED_MOTION_CACHE: AtomicU8 = AtomicU8::new(0);
static HIGH_CONTRAST_CACHE: AtomicU8 = AtomicU8::new(0);

type Waker = Box<dyn Fn() + Send + Sync>;
static WAKER: Mutex<Option<Waker>> = Mutex::new(None);

pub(crate) fn set_waker(waker: impl Fn() + Send + Sync + 'static) {
    if let Ok(mut slot) = WAKER.lock() {
        *slot = Some(Box::new(waker));
    }
    ensure_started();
}

pub(crate) fn reduced_motion() -> Option<bool> {
    ensure_started();
    decode_cache(REDUCED_MOTION_CACHE.load(Ordering::Acquire))
}

pub(crate) fn high_contrast() -> Option<bool> {
    ensure_started();
    decode_cache(HIGH_CONTRAST_CACHE.load(Ordering::Acquire))
}

fn decode_cache(state: u8) -> Option<bool> {
    match state {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

fn encode_cache(val: Option<bool>) -> u8 {
    match val {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

fn ensure_started() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Perform a bounded initial synchronous probe so first window creation
        // sees real portal values without racing the listener thread.
        if let Some((rm, hc)) = read_initial_settings() {
            REDUCED_MOTION_CACHE.store(encode_cache(rm), Ordering::Release);
            HIGH_CONTRAST_CACHE.store(encode_cache(hc), Ordering::Release);
        }
        let _ = std::thread::Builder::new()
            .name("nana-portal-settings".into())
            .spawn(listen_for_setting_changes);
    });
}

fn notify_waker() {
    if let Ok(slot) = WAKER.lock()
        && let Some(wake) = slot.as_ref()
    {
        wake();
    }
}

pub(crate) async fn dbus_timeout<T>(
    duration: Duration,
    fut: impl std::future::Future<Output = T>,
) -> Option<T> {
    let (tx, rx) = async_channel::bounded::<()>(1);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done_bg = std::sync::Arc::clone(&done);
    let handle = std::thread::Builder::new()
        .name("nana-dbus-timeout".into())
        .spawn(move || {
            let deadline = std::time::Instant::now() + duration;
            while !done_bg.load(std::sync::atomic::Ordering::Acquire) {
                let Some(rem) = deadline.checked_duration_since(std::time::Instant::now()) else {
                    break;
                };
                std::thread::park_timeout(rem);
            }
            if !done_bg.load(std::sync::atomic::Ordering::Acquire) {
                let _ = tx.try_send(());
            }
        })
        .ok();
    let result = future::or(async move { Some(fut.await) }, async move {
        let _ = rx.recv().await;
        None
    })
    .await;
    done.store(true, std::sync::atomic::Ordering::Release);
    if let Some(handle) = handle {
        handle.thread().unpark();
    }
    result
}

async fn with_timeout<T>(fut: impl std::future::Future<Output = T>) -> Option<T> {
    dbus_timeout(PORTAL_TIMEOUT, fut).await
}

fn read_initial_settings() -> Option<(Option<bool>, Option<bool>)> {
    future::block_on(async {
        let conn = with_timeout(zbus::Connection::session()).await?.ok()?;
        let dbus = with_timeout(zbus::fdo::DBusProxy::new(&conn)).await?.ok()?;
        let name = PORTAL_SERVICE.try_into().ok()?;
        let owned = with_timeout(dbus.name_has_owner(name)).await?.ok()?;
        if !owned {
            return None;
        }
        let proxy = with_timeout(zbus::Proxy::new(
            &conn,
            PORTAL_SERVICE,
            PORTAL_PATH,
            SETTINGS_IFACE,
        ))
        .await?
        .ok()?;
        let rm = read_one_bool(&proxy, KEY_REDUCED_MOTION).await;
        let hc = read_one_bool(&proxy, KEY_HIGH_CONTRAST).await;
        Some((rm, hc))
    })
}

async fn read_one_bool(proxy: &zbus::Proxy<'_>, key: &str) -> Option<bool> {
    let reply = with_timeout(proxy.call_method("ReadOne", &(APPEARANCE_NAMESPACE, key)))
        .await?
        .ok()?;
    let val: OwnedValue = reply.body().deserialize().ok()?;
    parse_portal_appearance_bool(&val)
}

fn parse_portal_appearance_bool(val: &OwnedValue) -> Option<bool> {
    if let Ok(num) = u32::try_from(val) {
        return Some(num == 1);
    }
    if let Ok(num) = i32::try_from(val) {
        return Some(num == 1);
    }
    if let Ok(flag) = bool::try_from(val) {
        return Some(flag);
    }
    if let zbus::zvariant::Value::Value(inner) = &**val {
        if let Ok(num) = u32::try_from(&**inner) {
            return Some(num == 1);
        }
        if let Ok(flag) = bool::try_from(&**inner) {
            return Some(flag);
        }
    }
    None
}

fn listen_for_setting_changes() {
    future::block_on(async {
        let Some(Ok(conn)) = with_timeout(zbus::Connection::session()).await else {
            return;
        };
        let Some(Ok(dbus)) = with_timeout(zbus::fdo::DBusProxy::new(&conn)).await else {
            return;
        };
        let Ok(name) = PORTAL_SERVICE.try_into() else {
            return;
        };
        if !with_timeout(dbus.name_has_owner(name))
            .await
            .and_then(Result::ok)
            .unwrap_or(false)
        {
            return;
        }
        let Some(Ok(proxy)) = with_timeout(zbus::Proxy::new(
            &conn,
            PORTAL_SERVICE,
            PORTAL_PATH,
            SETTINGS_IFACE,
        ))
        .await
        else {
            return;
        };
        let Ok(mut stream) = proxy.receive_signal("SettingChanged").await else {
            return;
        };
        while let Some(msg) = stream.next().await {
            let Ok((namespace, key, val)) =
                msg.body().deserialize::<(String, String, OwnedValue)>()
            else {
                continue;
            };
            if namespace != APPEARANCE_NAMESPACE {
                continue;
            }
            let Some(flag) = parse_portal_appearance_bool(&val) else {
                continue;
            };
            let encoded = encode_cache(Some(flag));
            match key.as_str() {
                KEY_REDUCED_MOTION => {
                    if REDUCED_MOTION_CACHE.swap(encoded, Ordering::AcqRel) != encoded {
                        crate::motion_preference::mark_changed();
                        notify_waker();
                    }
                }
                KEY_HIGH_CONTRAST => {
                    if HIGH_CONTRAST_CACHE.swap(encoded, Ordering::AcqRel) != encoded {
                        crate::contrast_preference::mark_changed();
                        notify_waker();
                    }
                }
                _ => {}
            }
        }
    });
}
