use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use futures_lite::future;
use futures_lite::stream::StreamExt;
use zbus::zvariant::{OwnedValue, Str, Structure, Value};

use super::{
    EventHub, NotificationBackendKind, NotificationDismissReason, NotificationError,
    NotificationEvent, NotificationOutcome, NotificationPermission, NotificationRequest,
    NotificationSupport, decode_png_freedesktop_image_data, timeout_to_freedesktop_ms,
};

const DBUS_TIMEOUT: Duration = Duration::from_millis(500);
const NOTIF_SERVICE: &str = "org.freedesktop.Notifications";
const NOTIF_PATH: &str = "/org/freedesktop/Notifications";
const NOTIF_IFACE: &str = "org.freedesktop.Notifications";

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct TrackedIds {
    /// Logical string id -> active freedesktop numeric id.
    by_logical: HashMap<String, u32>,
    /// Freedesktop numeric id -> logical string id.
    by_native: HashMap<u32, String>,
}

impl TrackedIds {
    fn new() -> Self {
        Self {
            by_logical: HashMap::new(),
            by_native: HashMap::new(),
        }
    }

    fn record(&mut self, logical: &str, native: u32) -> bool {
        let replaced = if let Some(prev) = self.by_logical.insert(logical.to_owned(), native) {
            if prev != native {
                self.by_native.remove(&prev);
            }
            true
        } else {
            false
        };
        self.by_native.insert(native, logical.to_owned());
        replaced
    }

    fn remove_by_native(&mut self, native: u32) -> Option<String> {
        let logical = self.by_native.remove(&native)?;
        if self.by_logical.get(&logical) == Some(&native) {
            self.by_logical.remove(&logical);
        }
        Some(logical)
    }

    fn remove_by_logical(&mut self, logical: &str) -> Option<u32> {
        let native = self.by_logical.remove(logical)?;
        self.by_native.remove(&native);
        Some(native)
    }
}

pub(super) struct LinuxNotificationBackend {
    hub: Arc<EventHub>,
    tracked: Arc<Mutex<TrackedIds>>,
    stop: Arc<AtomicBool>,
    listener: Mutex<Option<JoinHandle<()>>>,
}

impl LinuxNotificationBackend {
    pub(super) fn new(hub: Arc<EventHub>) -> Self {
        Self {
            hub,
            tracked: Arc::new(Mutex::new(TrackedIds::new())),
            stop: Arc::new(AtomicBool::new(false)),
            listener: Mutex::new(None),
        }
    }

    pub(super) fn support(&self) -> NotificationSupport {
        match probe_capabilities() {
            Ok(caps) => NotificationSupport {
                backend: NotificationBackendKind::FreedesktopNotifications,
                permission: NotificationPermission::Granted,
                supports_actions: caps.iter().any(|c| c == "actions"),
                supports_replace: true,
                supports_images: caps
                    .iter()
                    .any(|c| c == "body-images" || c == "icon-static" || c == "persistence"),
                unavailable_reason: None,
            },
            Err(reason) => NotificationSupport {
                backend: NotificationBackendKind::Unavailable,
                permission: NotificationPermission::Unsupported,
                supports_actions: false,
                supports_replace: false,
                supports_images: false,
                unavailable_reason: Some(reason),
            },
        }
    }

    pub(super) fn request_permission(&self) -> Result<NotificationPermission, NotificationError> {
        match self.support().backend {
            NotificationBackendKind::Unavailable => Err(NotificationError::Unavailable(
                "org.freedesktop.Notifications is unavailable".into(),
            )),
            _ => Ok(NotificationPermission::Granted),
        }
    }

    pub(super) fn present(
        &self,
        request: NotificationRequest,
    ) -> Result<NotificationOutcome, NotificationError> {
        let logical_id = request
            .tag
            .clone()
            .unwrap_or_else(|| format!("nana-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
        self.send(&logical_id, request)
    }

    pub(super) fn update(
        &self,
        id: &str,
        request: NotificationRequest,
    ) -> Result<NotificationOutcome, NotificationError> {
        self.send(id, request)
    }

    pub(super) fn withdraw(&self, id: &str) -> Result<(), NotificationError> {
        let native_id = self
            .tracked
            .lock()
            .ok()
            .and_then(|mut map| map.remove_by_logical(id));
        let Some(native_id) = native_id else {
            return Ok(());
        };
        future::block_on(async move {
            let conn = connect_and_check_owner().await?;
            let proxy = notif_proxy(&conn).await?;
            with_timeout(
                proxy.call_method("CloseNotification", &(native_id,)),
                "CloseNotification",
            )
            .await?
            .map_err(|e| NotificationError::Platform(e.to_string()))?;
            Ok(())
        })
    }

    pub(super) fn clear(&self) -> Result<(), NotificationError> {
        let native_ids: Vec<u32> = self
            .tracked
            .lock()
            .map(|mut map| {
                let ids: Vec<u32> = map.by_native.keys().copied().collect();
                map.by_logical.clear();
                map.by_native.clear();
                ids
            })
            .unwrap_or_default();
        if native_ids.is_empty() {
            return Ok(());
        }
        future::block_on(async move {
            let conn = connect_and_check_owner().await?;
            let proxy = notif_proxy(&conn).await?;
            for id in native_ids {
                let _ = with_timeout(
                    proxy.call_method("CloseNotification", &(id,)),
                    "CloseNotification",
                )
                .await;
            }
            Ok(())
        })
    }

    fn ensure_listener(&self) {
        let Ok(mut guard) = self.listener.lock() else {
            return;
        };
        if guard.is_some() {
            return;
        }
        let hub = Arc::clone(&self.hub);
        let tracked = Arc::clone(&self.tracked);
        let stop = Arc::clone(&self.stop);
        let (ready_tx, ready_rx) = mpsc::channel();
        if let Ok(handle) = std::thread::Builder::new()
            .name("nana-linux-notifications".into())
            .spawn(move || run_signal_listener(hub, tracked, stop, ready_tx))
        {
            let _ = ready_rx.recv_timeout(DBUS_TIMEOUT);
            *guard = Some(handle);
        }
    }

    fn send(
        &self,
        logical_id: &str,
        request: NotificationRequest,
    ) -> Result<NotificationOutcome, NotificationError> {
        self.ensure_listener();
        let replaces_id = self
            .tracked
            .lock()
            .ok()
            .and_then(|map| map.by_logical.get(logical_id).copied())
            .unwrap_or(0);

        let image_data = request
            .image_png
            .as_deref()
            .map(decode_png_freedesktop_image_data)
            .transpose()?;

        let native_id = future::block_on(async move {
            let conn = connect_and_check_owner().await?;
            let proxy = notif_proxy(&conn).await?;

            let app_name = request.app_name.as_deref().unwrap_or("NanaUI").to_owned();
            let app_icon = String::new();
            let mut actions: Vec<String> = Vec::with_capacity(request.actions.len() * 2 + 2);
            actions.push("default".into());
            actions.push("Open".into());
            for act in &request.actions {
                actions.push(act.id.clone());
                actions.push(act.label.clone());
            }

            let mut hints: HashMap<String, OwnedValue> = HashMap::new();
            if request.silent {
                hints.insert("suppress-sound".into(), OwnedValue::from(true));
            }
            if let Some(tag) = request.tag.as_deref() {
                hints.insert("x-kde-origin-name".into(), OwnedValue::from(Str::from(tag)));
            }
            if let Some((w, h, stride, has_alpha, bps, ch, data)) = image_data {
                let structure = Structure::from((w, h, stride, has_alpha, bps, ch, data));
                if let Ok(val) = Value::Structure(structure).try_to_owned() {
                    hints.insert("image-data".into(), val);
                }
            }

            let expire_timeout = timeout_to_freedesktop_ms(request.timeout);
            let reply = with_timeout(
                proxy.call_method(
                    "Notify",
                    &(
                        app_name,
                        replaces_id,
                        app_icon,
                        request.title,
                        request.body,
                        actions,
                        hints,
                        expire_timeout,
                    ),
                ),
                "org.freedesktop.Notifications.Notify",
            )
            .await?
            .map_err(|e| NotificationError::Platform(e.to_string()))?;

            reply
                .body()
                .deserialize::<u32>()
                .map_err(|e| NotificationError::Platform(format!("invalid Notify reply: {e}")))
        })?;

        let replaced = self
            .tracked
            .lock()
            .map(|mut map| map.record(logical_id, native_id))
            .unwrap_or(replaces_id != 0);

        Ok(NotificationOutcome {
            id: logical_id.to_owned(),
            replaced,
            backend: NotificationBackendKind::FreedesktopNotifications,
        })
    }
}

impl Drop for LinuxNotificationBackend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.listener.lock()
            && let Some(handle) = guard.take()
        {
            drop(handle);
        }
    }
}

async fn with_timeout<T>(
    fut: impl std::future::Future<Output = T>,
    label: &str,
) -> Result<T, NotificationError> {
    crate::portal_settings_linux::dbus_timeout(DBUS_TIMEOUT, fut)
        .await
        .ok_or_else(|| NotificationError::Unavailable(format!("{label} timed out")))
}

async fn connect_and_check_owner() -> Result<zbus::Connection, NotificationError> {
    let conn = with_timeout(zbus::Connection::session(), "session D-Bus connect")
        .await?
        .map_err(|e| NotificationError::Unavailable(format!("session D-Bus unavailable: {e}")))?;
    let dbus = with_timeout(
        zbus::fdo::DBusProxy::new(&conn),
        "org.freedesktop.DBus proxy",
    )
    .await?
    .map_err(|e| NotificationError::Unavailable(format!("DBus proxy failed: {e}")))?;
    let owned = with_timeout(
        dbus.name_has_owner(
            NOTIF_SERVICE
                .try_into()
                .map_err(|e| NotificationError::Platform(format!("invalid service name: {e}")))?,
        ),
        "NameHasOwner(org.freedesktop.Notifications)",
    )
    .await?
    .map_err(|e| NotificationError::Unavailable(format!("NameHasOwner failed: {e}")))?;
    if !owned {
        return Err(NotificationError::Unavailable(
            "org.freedesktop.Notifications service is unavailable".into(),
        ));
    }
    Ok(conn)
}

async fn notif_proxy(conn: &zbus::Connection) -> Result<zbus::Proxy<'_>, NotificationError> {
    with_timeout(
        zbus::Proxy::new(conn, NOTIF_SERVICE, NOTIF_PATH, NOTIF_IFACE),
        "org.freedesktop.Notifications proxy",
    )
    .await?
    .map_err(|e| NotificationError::Platform(e.to_string()))
}

fn probe_capabilities() -> Result<Vec<String>, String> {
    future::block_on(async {
        let conn = connect_and_check_owner().await.map_err(|e| e.to_string())?;
        let proxy = notif_proxy(&conn).await.map_err(|e| e.to_string())?;
        let reply = with_timeout(proxy.call_method("GetCapabilities", &()), "GetCapabilities")
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        reply
            .body()
            .deserialize::<Vec<String>>()
            .map_err(|e| e.to_string())
    })
}

fn run_signal_listener(
    hub: Arc<EventHub>,
    tracked: Arc<Mutex<TrackedIds>>,
    stop: Arc<AtomicBool>,
    ready_tx: mpsc::Sender<()>,
) {
    future::block_on(async move {
        let Ok(conn) = connect_and_check_owner().await else {
            let _ = ready_tx.send(());
            return;
        };
        let Ok(proxy) = notif_proxy(&conn).await else {
            let _ = ready_tx.send(());
            return;
        };
        let Ok(mut action_stream) = proxy.receive_signal("ActionInvoked").await else {
            let _ = ready_tx.send(());
            return;
        };
        let Ok(mut closed_stream) = proxy.receive_signal("NotificationClosed").await else {
            let _ = ready_tx.send(());
            return;
        };
        let _ = ready_tx.send(());

        enum Msg {
            Action(u32, String),
            Closed(u32, u32),
            Done,
        }

        while !stop.load(Ordering::Relaxed) {
            let next = future::or(
                async {
                    match action_stream.next().await {
                        Some(msg) => match msg.body().deserialize::<(u32, String)>() {
                            Ok((id, key)) => Msg::Action(id, key),
                            Err(_) => Msg::Done,
                        },
                        None => Msg::Done,
                    }
                },
                async {
                    match closed_stream.next().await {
                        Some(msg) => match msg.body().deserialize::<(u32, u32)>() {
                            Ok((id, reason)) => Msg::Closed(id, reason),
                            Err(_) => Msg::Done,
                        },
                        None => Msg::Done,
                    }
                },
            )
            .await;

            match next {
                Msg::Action(native_id, key) => {
                    let logical = tracked
                        .lock()
                        .ok()
                        .and_then(|map| map.by_native.get(&native_id).cloned())
                        .unwrap_or_else(|| format!("freedesktop-{native_id}"));
                    let action_id = if key == "default" || key.is_empty() {
                        None
                    } else {
                        Some(key)
                    };
                    hub.push(NotificationEvent::Activated {
                        id: logical,
                        action_id,
                    });
                }
                Msg::Closed(native_id, reason_code) => {
                    let logical = tracked
                        .lock()
                        .ok()
                        .and_then(|mut map| map.remove_by_native(native_id))
                        .unwrap_or_else(|| format!("freedesktop-{native_id}"));
                    let reason = match reason_code {
                        1 => NotificationDismissReason::Expired,
                        2 => NotificationDismissReason::User,
                        3 => NotificationDismissReason::Closed,
                        _ => NotificationDismissReason::Unknown,
                    };
                    hub.push(NotificationEvent::Dismissed {
                        id: logical,
                        reason,
                    });
                }
                Msg::Done => break,
            }
        }
    });
}
