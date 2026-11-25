use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use futures_lite::future;
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Structure, Value};

use super::{
    Tray, TrayAttention, decode_png_rgba, rgba_to_sni_argb32, select_tray_icon_bytes,
    summarize_menu_entries,
};

const DBUS_TIMEOUT: Duration = Duration::from_millis(500);
const SNI_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

static ITEM_COUNTER: AtomicU64 = AtomicU64::new(1);

enum Command {
    Update(Tray, mpsc::Sender<Result<(), String>>),
    SetAttention(TrayAttention, mpsc::Sender<bool>),
    Stop,
}

#[derive(Clone)]
struct SniSnapshot {
    id: String,
    tray: Tray,
    revision: u32,
    pixmap: Option<(i32, i32, Vec<u8>)>,
    menu_entries: Vec<DbusMenuItem>,
}

#[derive(Clone)]
struct DbusMenuItem {
    id: i32,
    command_id: Option<u32>,
    label: String,
    is_separator: bool,
    enabled: bool,
    check_state: Option<bool>,
    children: Vec<DbusMenuItem>,
}

struct LinuxTrayRuntime {
    tx: Sender<Command>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<Mutex<SniSnapshot>>,
}

static STATE: Mutex<Option<LinuxTrayRuntime>> = Mutex::new(None);

pub(super) fn install(tray: &Tray) -> Result<(), String> {
    let mut guard = STATE
        .lock()
        .map_err(|_| "tray state lock poisoned".to_string())?;
    if let Some(runtime) = guard.as_ref() {
        let (reply_tx, reply_rx) = mpsc::channel();
        if runtime
            .tx
            .try_send(Command::Update(tray.clone(), reply_tx))
            .is_ok()
            && let Ok(res) = reply_rx.recv_timeout(DBUS_TIMEOUT)
        {
            return res;
        }
        // Existing background worker died; replace it below.
    }
    if let Some(mut stale) = guard.take() {
        let _ = stale.tx.try_send(Command::Stop);
        if let Some(handle) = stale.thread.take() {
            let _ = handle.join();
        }
    }

    let snapshot = build_snapshot(
        format!(
            "nana-tray-{}-{}",
            std::process::id(),
            ITEM_COUNTER.fetch_add(1, Ordering::Relaxed)
        ),
        tray.clone(),
        1,
    )?;
    let shared = Arc::new(Mutex::new(snapshot));
    let (tx, rx) = async_channel::bounded::<Command>(32);
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let thread_shared = Arc::clone(&shared);
    let handle = std::thread::Builder::new()
        .name("nana-linux-tray".into())
        .spawn(move || run_tray_service(rx, thread_shared, ready_tx))
        .map_err(|error| format!("failed to spawn Linux tray thread: {error}"))?;

    match ready_rx.recv_timeout(DBUS_TIMEOUT.saturating_mul(2)) {
        Ok(Ok(())) => {
            *guard = Some(LinuxTrayRuntime {
                tx,
                thread: Some(handle),
                shared,
            });
            Ok(())
        }
        Ok(Err(reason)) => {
            let _ = tx.try_send(Command::Stop);
            let _ = handle.join();
            Err(reason)
        }
        Err(_) => {
            let _ = tx.try_send(Command::Stop);
            Err("timed out starting StatusNotifierItem on session D-Bus".into())
        }
    }
}

pub(super) fn set_attention(attention: TrayAttention) -> bool {
    let Ok(guard) = STATE.lock() else {
        return false;
    };
    let Some(runtime) = guard.as_ref() else {
        return false;
    };
    let (tx, rx) = mpsc::channel();
    if runtime
        .tx
        .try_send(Command::SetAttention(attention, tx))
        .is_err()
    {
        return false;
    }
    rx.recv_timeout(DBUS_TIMEOUT).unwrap_or(false)
}

pub(super) fn installed() -> Option<(String, Vec<String>)> {
    let guard = STATE.lock().ok()?;
    let runtime = guard.as_ref()?;
    let snap = runtime.shared.lock().ok()?;
    Some((
        snap.tray.tooltip.clone(),
        summarize_menu_entries(&snap.tray.menu),
    ))
}

pub(super) fn remove() {
    let Ok(mut guard) = STATE.lock() else {
        return;
    };
    if let Some(mut runtime) = guard.take() {
        let _ = runtime.tx.try_send(Command::Stop);
        if let Some(handle) = runtime.thread.take() {
            let _ = handle.join();
        }
    }
}

fn build_snapshot(id: String, tray: Tray, revision: u32) -> Result<SniSnapshot, String> {
    // Panel backgrounds on GNOME/KDE are usually dark unless overridden;
    // select light glyph when explicit variant or template mask exists.
    let light_panel = false;
    let selected = select_tray_icon_bytes(&tray, light_panel);
    let pixmap = if selected.is_empty() {
        None
    } else {
        let (w, h, rgba) = decode_png_rgba(selected.as_ref())
            .ok_or_else(|| "invalid tray PNG icon bytes".to_string())?;
        Some((w as i32, h as i32, rgba_to_sni_argb32(&rgba)))
    };
    let mut next_id = 1i32;
    let menu_entries = convert_menu_entries(&tray.menu, &mut next_id);
    Ok(SniSnapshot {
        id,
        tray,
        revision,
        pixmap,
        menu_entries,
    })
}

fn convert_menu_entries(entries: &[crate::MenuEntry], next_id: &mut i32) -> Vec<DbusMenuItem> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let node_id = *next_id;
        *next_id = next_id.saturating_add(1);
        match entry {
            crate::MenuEntry::Separator => out.push(DbusMenuItem {
                id: node_id,
                command_id: None,
                label: String::new(),
                is_separator: true,
                enabled: true,
                check_state: None,
                children: Vec::new(),
            }),
            crate::MenuEntry::Item {
                id,
                label,
                enabled,
                checked,
                ..
            } => out.push(DbusMenuItem {
                id: node_id,
                command_id: Some(*id),
                label: label.clone(),
                is_separator: false,
                enabled: *enabled,
                check_state: Some(*checked),
                children: Vec::new(),
            }),
            crate::MenuEntry::Submenu(menu) => {
                let children = convert_menu_entries(&menu.entries, next_id);
                out.push(DbusMenuItem {
                    id: node_id,
                    command_id: None,
                    label: menu.title.clone(),
                    is_separator: false,
                    enabled: true,
                    check_state: None,
                    children,
                });
            }
        }
    }
    out
}

fn find_item<'a>(items: &'a [DbusMenuItem], target_id: i32) -> Option<&'a DbusMenuItem> {
    for item in items {
        if item.id == target_id {
            return Some(item);
        }
        if let Some(found) = find_item(&item.children, target_id) {
            return Some(found);
        }
    }
    None
}

fn item_properties(item: &DbusMenuItem, filter: &[String]) -> HashMap<String, OwnedValue> {
    let want = |name: &str| filter.is_empty() || filter.iter().any(|key| key == name);
    let mut map = HashMap::new();
    if item.is_separator {
        if want("type") {
            map.insert("type".to_owned(), OwnedValue::from(Str::from("separator")));
        }
        if want("visible") {
            map.insert("visible".to_owned(), OwnedValue::from(true));
        }
        return map;
    }
    if want("label") {
        map.insert(
            "label".to_owned(),
            OwnedValue::from(Str::from(item.label.as_str())),
        );
    }
    if want("enabled") {
        map.insert("enabled".to_owned(), OwnedValue::from(item.enabled));
    }
    if want("visible") {
        map.insert("visible".to_owned(), OwnedValue::from(true));
    }
    if !item.children.is_empty() && want("children-display") {
        map.insert(
            "children-display".to_owned(),
            OwnedValue::from(Str::from("submenu")),
        );
    }
    if let Some(checked) = item.check_state {
        if want("toggle-type") {
            map.insert(
                "toggle-type".to_owned(),
                OwnedValue::from(Str::from("checkmark")),
            );
        }
        if want("toggle-state") {
            map.insert(
                "toggle-state".to_owned(),
                OwnedValue::from(if checked { 1i32 } else { 0i32 }),
            );
        }
    }
    map
}

fn menu_node_to_owned_value(
    item: &DbusMenuItem,
    remaining_depth: i32,
    filter: &[String],
) -> OwnedValue {
    let props = item_properties(item, filter);
    let children: Vec<OwnedValue> = if remaining_depth == 0 {
        Vec::new()
    } else {
        let next_depth = if remaining_depth < 0 {
            -1
        } else {
            remaining_depth - 1
        };
        item.children
            .iter()
            .map(|child| menu_node_to_owned_value(child, next_depth, filter))
            .collect()
    };
    let structure = Structure::from((item.id, props, children));
    Value::Structure(structure)
        .try_to_owned()
        .unwrap_or_else(|_| OwnedValue::from(0i32))
}

struct StatusNotifierItemIface {
    shared: Arc<Mutex<SniSnapshot>>,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItemIface {
    #[zbus(property)]
    async fn category(&self) -> &'static str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    async fn id(&self) -> String {
        self.shared
            .lock()
            .map(|s| s.id.clone())
            .unwrap_or_else(|_| "nana-tray".into())
    }

    #[zbus(property)]
    async fn title(&self) -> String {
        self.shared
            .lock()
            .map(|s| s.tray.tooltip.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn status(&self) -> &'static str {
        self.shared
            .lock()
            .map(|s| s.tray.attention.as_sni_status())
            .unwrap_or("Active")
    }

    #[zbus(property)]
    async fn icon_name(&self) -> &'static str {
        ""
    }

    #[zbus(property)]
    async fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        self.shared
            .lock()
            .ok()
            .and_then(|s| s.pixmap.clone())
            .into_iter()
            .collect()
    }

    #[zbus(property)]
    async fn attention_icon_name(&self) -> &'static str {
        ""
    }

    #[zbus(property)]
    async fn attention_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        self.shared
            .lock()
            .ok()
            .filter(|s| matches!(s.tray.attention, TrayAttention::NeedsAttention))
            .and_then(|s| s.pixmap.clone())
            .into_iter()
            .collect()
    }

    #[zbus(property)]
    async fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        let title = self
            .shared
            .lock()
            .map(|s| s.tray.tooltip.clone())
            .unwrap_or_default();
        (String::new(), Vec::new(), title, String::new())
    }

    #[zbus(property)]
    async fn item_is_menu(&self) -> bool {
        self.shared
            .lock()
            .map(|s| s.tray.primary.is_none())
            .unwrap_or(true)
    }

    #[zbus(property)]
    async fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(MENU_PATH).expect("valid static menu path")
    }

    async fn activate(&self, _x: i32, _y: i32) {
        let primary = self.shared.lock().ok().and_then(|s| s.tray.primary);
        if let Some(id) = primary {
            crate::menu::push_activation(id);
            super::notify();
        }
    }

    async fn secondary_activate(&self, _x: i32, _y: i32) {}

    async fn context_menu(&self, _x: i32, _y: i32) {}

    async fn scroll(&self, _delta: i32, _orientation: &str) {}

    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

struct DbusMenuIface {
    shared: Arc<Mutex<SniSnapshot>>,
}

#[interface(name = "com.canonical.dbusmenu")]
impl DbusMenuIface {
    #[zbus(property)]
    async fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    async fn text_direction(&self) -> &'static str {
        "ltr"
    }

    #[zbus(property)]
    async fn status(&self) -> &'static str {
        "normal"
    }

    async fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)) {
        let Ok(snap) = self.shared.lock() else {
            return (0, (parent_id, HashMap::new(), Vec::new()));
        };
        let (root_id, props, children_slice): (i32, HashMap<String, OwnedValue>, &[DbusMenuItem]) =
            if parent_id == 0 {
                let mut root_props = HashMap::new();
                if !snap.menu_entries.is_empty() {
                    root_props.insert(
                        "children-display".to_owned(),
                        OwnedValue::from(Str::from("submenu")),
                    );
                }
                (0, root_props, &snap.menu_entries)
            } else if let Some(item) = find_item(&snap.menu_entries, parent_id) {
                (
                    item.id,
                    item_properties(item, &property_names),
                    &item.children,
                )
            } else {
                (parent_id, HashMap::new(), &[])
            };
        let child_values = if recursion_depth == 0 {
            Vec::new()
        } else {
            let next_depth = if recursion_depth < 0 {
                -1
            } else {
                recursion_depth - 1
            };
            children_slice
                .iter()
                .map(|child| menu_node_to_owned_value(child, next_depth, &property_names))
                .collect()
        };
        (snap.revision, (root_id, props, child_values))
    }

    async fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let Ok(snap) = self.shared.lock() else {
            return Vec::new();
        };
        ids.into_iter()
            .filter_map(|id| {
                find_item(&snap.menu_entries, id)
                    .map(|item| (id, item_properties(item, &property_names)))
            })
            .collect()
    }

    async fn get_property(&self, id: i32, name: &str) -> OwnedValue {
        let Ok(snap) = self.shared.lock() else {
            return OwnedValue::from(Str::from(""));
        };
        let filter = vec![name.to_owned()];
        find_item(&snap.menu_entries, id)
            .and_then(|item| item_properties(item, &filter).remove(name))
            .unwrap_or_else(|| OwnedValue::from(Str::from("")))
    }

    async fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id == "clicked"
            && let Ok(snap) = self.shared.lock()
            && let Some(item) = find_item(&snap.menu_entries, id)
            && let Some(command_id) = item.command_id
        {
            drop(snap);
            crate::menu::push_activation(command_id);
            super::notify();
        }
    }

    async fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        let mut unknown = Vec::new();
        for (id, event_id, _, _) in events {
            let mut matched = false;
            if let Ok(snap) = self.shared.lock()
                && let Some(item) = find_item(&snap.menu_entries, id)
            {
                matched = true;
                if event_id == "clicked"
                    && let Some(command_id) = item.command_id
                {
                    drop(snap);
                    crate::menu::push_activation(command_id);
                    super::notify();
                }
            }
            if !matched {
                unknown.push(id);
            }
        }
        unknown
    }

    async fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    async fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

async fn with_timeout<T>(
    fut: impl std::future::Future<Output = T>,
    label: &str,
) -> Result<T, String> {
    crate::portal_settings_linux::dbus_timeout(DBUS_TIMEOUT, fut)
        .await
        .ok_or_else(|| format!("{label} timed out"))
}

async fn setup_dbus(shared: &Arc<Mutex<SniSnapshot>>) -> Result<zbus::Connection, String> {
    let bus_name = shared
        .lock()
        .map(|s| format!("org.kde.StatusNotifierItem-{}", s.id))
        .unwrap_or_else(|_| format!("org.kde.StatusNotifierItem-nana-{}", std::process::id()));

    let probe = with_timeout(zbus::Connection::session(), "session D-Bus connect")
        .await?
        .map_err(|e| format!("session D-Bus unavailable: {e}"))?;

    let dbus_proxy = with_timeout(
        zbus::fdo::DBusProxy::new(&probe),
        "org.freedesktop.DBus proxy",
    )
    .await?
    .map_err(|e| format!("org.freedesktop.DBus proxy failed: {e}"))?;

    let watcher_present = with_timeout(
        dbus_proxy.name_has_owner(
            WATCHER_SERVICE
                .try_into()
                .map_err(|e| format!("invalid watcher bus name: {e}"))?,
        ),
        "StatusNotifierWatcher check",
    )
    .await?
    .map_err(|e| format!("StatusNotifierWatcher lookup failed: {e}"))?;

    if !watcher_present {
        return Err("StatusNotifierWatcher is unavailable on session D-Bus".into());
    }
    drop(probe);

    let sni = StatusNotifierItemIface {
        shared: Arc::clone(shared),
    };
    let menu = DbusMenuIface {
        shared: Arc::clone(shared),
    };

    let conn = with_timeout(
        zbus::connection::Builder::session()
            .map_err(|e| format!("session bus builder failed: {e}"))?
            .name(bus_name.as_str())
            .map_err(|e| format!("SNI well-known name failed: {e}"))?
            .serve_at(SNI_PATH, sni)
            .map_err(|e| format!("serve SNI object failed: {e}"))?
            .serve_at(MENU_PATH, menu)
            .map_err(|e| format!("serve DBusMenu object failed: {e}"))?
            .build(),
        "SNI connection build",
    )
    .await?
    .map_err(|e| format!("SNI connection failed: {e}"))?;

    let watcher = with_timeout(
        zbus::Proxy::new(
            &conn,
            WATCHER_SERVICE,
            WATCHER_PATH,
            "org.kde.StatusNotifierWatcher",
        ),
        "StatusNotifierWatcher proxy",
    )
    .await?
    .map_err(|e| format!("StatusNotifierWatcher proxy failed: {e}"))?;

    with_timeout(
        watcher.call_method("RegisterStatusNotifierItem", &(bus_name.as_str(),)),
        "RegisterStatusNotifierItem",
    )
    .await?
    .map_err(|e| format!("RegisterStatusNotifierItem failed: {e}"))?;

    Ok(conn)
}

fn run_tray_service(
    rx: Receiver<Command>,
    shared: Arc<Mutex<SniSnapshot>>,
    ready_tx: mpsc::Sender<Result<(), String>>,
) {
    future::block_on(async move {
        let conn = match setup_dbus(&shared).await {
            Ok(conn) => {
                let _ = ready_tx.send(Ok(()));
                conn
            }
            Err(reason) => {
                let _ = ready_tx.send(Err(reason));
                return;
            }
        };

        while let Ok(cmd) = rx.recv().await {
            match cmd {
                Command::Stop => break,
                Command::Update(tray, reply) => {
                    let result = update_snapshot(&shared, &conn, tray).await;
                    let _ = reply.send(result);
                }
                Command::SetAttention(attention, reply) => {
                    let updated = update_attention(&shared, &conn, attention).await;
                    let _ = reply.send(updated);
                }
            }
        }
        let _ = conn.close().await;
    });
}

async fn update_snapshot(
    shared: &Arc<Mutex<SniSnapshot>>,
    conn: &zbus::Connection,
    tray: Tray,
) -> Result<(), String> {
    let (id, next_rev, status_str) = {
        let guard = shared
            .lock()
            .map_err(|_| "tray state lock poisoned".to_string())?;
        (
            guard.id.clone(),
            guard.revision.saturating_add(1),
            tray.attention.as_sni_status(),
        )
    };
    let next = build_snapshot(id, tray, next_rev)?;
    if let Ok(mut guard) = shared.lock() {
        *guard = next;
    }
    if let Ok(sni_iface) = conn
        .object_server()
        .interface::<_, StatusNotifierItemIface>(SNI_PATH)
        .await
    {
        let emitter = sni_iface.signal_emitter();
        let _ = StatusNotifierItemIface::new_icon(emitter).await;
        let _ = StatusNotifierItemIface::new_title(emitter).await;
        let _ = StatusNotifierItemIface::new_tool_tip(emitter).await;
        let _ = StatusNotifierItemIface::new_status(emitter, status_str).await;
    }
    if let Ok(menu_iface) = conn
        .object_server()
        .interface::<_, DbusMenuIface>(MENU_PATH)
        .await
    {
        let _ = DbusMenuIface::layout_updated(menu_iface.signal_emitter(), next_rev, 0).await;
    }
    Ok(())
}

async fn update_attention(
    shared: &Arc<Mutex<SniSnapshot>>,
    conn: &zbus::Connection,
    attention: TrayAttention,
) -> bool {
    if let Ok(mut guard) = shared.lock() {
        guard.tray.attention = attention;
    } else {
        return false;
    }
    if let Ok(sni_iface) = conn
        .object_server()
        .interface::<_, StatusNotifierItemIface>(SNI_PATH)
        .await
    {
        let _ = StatusNotifierItemIface::new_status(
            sni_iface.signal_emitter(),
            attention.as_sni_status(),
        )
        .await;
    }
    true
}
