//! **L1** progressive `window` / `document` / Web API compatibility for NanaUI Vue.
//!
//! This is a buffered WebView-source compatibility subset, not a WebView or a
//! second paint path. JavaScript runs in the Rust-owned V8 engine and all
//! visible output still maps through NanaUI Runtime/UiScene to `SceneWgpuPainter`.

#![allow(clippy::field_reassign_with_default)]

mod canvas;
mod fetch;
mod media;
mod ws;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nana_js_engine::{HostApiRegistry, HostValue, JsException, RuntimeArtifact};

pub use canvas::{
    CanvasBitmap, CanvasError, CanvasId, CanvasResourceKind, CanvasRuntime, CanvasUpload,
    SharedCanvasRuntime, shared_canvas_runtime,
};
use fetch::{FetchCompletion, FetchRuntime};
pub use media::{
    MediaCaptureMode, MediaError, MediaId, MediaKind, MediaLiveSets, MediaRuntime, MediaStreamId,
    MediaTreeRef, MediaUpload, SharedMediaRuntime, media_live_sets_from_tree,
    released_media_gpu_slots, shared_media_runtime,
};
use ws::{SocketEvent, SocketRuntime};

/// Fallback gap between host frames while rAF is pending. `pump_frame` consumes
/// pending rAF for the current host frame; this interval is `next_wakeup`, not a
/// spin inside the drain loop.
const RAF_FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[cfg(not(target_os = "android"))]
pub use nana_ui_platform::OsClipboard;
pub use nana_ui_platform::{
    ClipboardHost, FetchError, FetchErrorKind, FetchHost, FetchPolicy, FetchRequest, FetchResponse,
    MemoryClipboard, NativeFetchHost, SharedClipboardHost, SharedFetchHost, SharedWebSocketHost,
    SocketPolicy, UnsupportedClipboard, WebSocketHost, WsError, WsErrorKind, WsEvent, WsMessage,
    WsOpenRequest, WsSink, default_shared_clipboard, shared_clipboard, shared_fetch_host,
};

/// UTF-8 JS that installs window/document/localStorage/rAF/history/… on `globalThis`.
pub const WEB_API_SHIM_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/shim.js"));

pub fn shim_source() -> &'static str {
    WEB_API_SHIM_JS
}

pub fn shim_artifact() -> RuntimeArtifact {
    RuntimeArtifact::from_source("nana-web-api-shim.js", WEB_API_SHIM_JS)
}

/// Compose web-api shim + app bundle into one [`RuntimeArtifact`].
pub fn compose_runtime_artifact(name: impl Into<String>, app_source: &str) -> RuntimeArtifact {
    let mut bytes = String::with_capacity(WEB_API_SHIM_JS.len() + app_source.len() + 8);
    bytes.push_str(WEB_API_SHIM_JS);
    bytes.push('\n');
    bytes.push_str(app_source);
    RuntimeArtifact::from_source(name, bytes)
}

/// In-memory Web API host state shared with JS via `__nanaHost`.
#[derive(Debug)]
pub struct WebApiState {
    local_storage: SharedStorage,
    storage: HashMap<String, BTreeMap<String, String>>,
    pending_raf: BTreeSet<u64>,
    raf_deadline: Option<Instant>,
    /// True between [`Self::begin_host_frame`] and [`Self::end_host_frame`].
    /// Nested rAF scheduled during a host frame waits for the next wakeup.
    host_frame_open: bool,
    timeouts: BTreeMap<u64, Instant>,
    intervals: BTreeMap<u64, (Instant, Duration)>,
    document_dataset: BTreeMap<String, String>,
    document_style: BTreeMap<String, String>,
    location_path: String,
    location_search: String,
    location_hash: String,
    fetch: FetchRuntime,
    socket: SocketRuntime,
}

impl Default for WebApiState {
    fn default() -> Self {
        Self::new()
    }
}

impl WebApiState {
    pub fn new() -> Self {
        Self::with_fetch_host(shared_fetch_host(NativeFetchHost::new(
            FetchPolicy::default(),
        )))
    }

    pub fn with_fetch_host(fetch_host: SharedFetchHost) -> Self {
        Self::with_fetch_host_and_local_storage(fetch_host, shared_storage())
    }

    pub fn with_fetch_host_and_local_storage(
        fetch_host: SharedFetchHost,
        local_storage: SharedStorage,
    ) -> Self {
        Self {
            location_path: "/".into(),
            local_storage,
            storage: HashMap::new(),
            pending_raf: BTreeSet::new(),
            raf_deadline: None,
            host_frame_open: false,
            timeouts: BTreeMap::new(),
            intervals: BTreeMap::new(),
            document_dataset: BTreeMap::new(),
            document_style: BTreeMap::new(),
            location_search: String::new(),
            location_hash: String::new(),
            fetch: FetchRuntime::new(fetch_host),
            socket: SocketRuntime::new(),
        }
    }

    /// Attach or detach the application-owned WebSocket transport. Absent by
    /// default: without a host the JS `WebSocket` surface reports unavailable.
    pub fn set_socket_host(&mut self, socket_host: Option<SharedWebSocketHost>) {
        self.socket.set_host(socket_host);
    }

    pub fn storage_get(&self, bucket: &str, key: &str) -> Option<String> {
        if bucket == "local" {
            return self
                .local_storage
                .lock()
                .ok()
                .and_then(|storage| storage.get(key).cloned());
        }
        self.storage.get(bucket).and_then(|m| m.get(key).cloned())
    }

    pub fn storage_set(&mut self, bucket: &str, key: &str, value: String) {
        if bucket == "local" {
            if let Ok(mut storage) = self.local_storage.lock() {
                storage.insert(key.to_string(), value);
            }
            return;
        }
        self.storage
            .entry(bucket.to_string())
            .or_default()
            .insert(key.to_string(), value);
    }

    pub fn storage_remove(&mut self, bucket: &str, key: &str) {
        if bucket == "local" {
            if let Ok(mut storage) = self.local_storage.lock() {
                storage.remove(key);
            }
            return;
        }
        if let Some(map) = self.storage.get_mut(bucket) {
            map.remove(key);
        }
    }

    pub fn storage_clear(&mut self, bucket: &str) {
        if bucket == "local" {
            if let Ok(mut storage) = self.local_storage.lock() {
                storage.clear();
            }
            return;
        }
        if let Some(map) = self.storage.get_mut(bucket) {
            map.clear();
        }
    }

    pub fn storage_keys(&self, bucket: &str) -> Vec<String> {
        if bucket == "local" {
            return self
                .local_storage
                .lock()
                .map(|storage| storage.keys().cloned().collect())
                .unwrap_or_default();
        }
        self.storage
            .get(bucket)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn document_dataset(&self) -> &BTreeMap<String, String> {
        &self.document_dataset
    }

    /// Write a `documentElement.dataset` entry (same store as `documentElementSet`).
    pub fn set_document_dataset(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.document_dataset.insert(key.into(), value.into());
    }

    pub fn document_style(&self) -> &BTreeMap<String, String> {
        &self.document_style
    }

    pub fn location_path(&self) -> &str {
        &self.location_path
    }

    /// Mark pending rAF due for this host frame so `pump_frame` follows the host
    /// rather than a wall-clock 16ms spin.
    pub fn begin_host_frame(&mut self, now: Instant) {
        self.host_frame_open = true;
        if !self.pending_raf.is_empty() {
            self.raf_deadline = Some(now);
        }
    }

    /// Close the host frame. Nested rAF waits until `next_wakeup` (~16ms).
    pub fn end_host_frame(&mut self, now: Instant) {
        self.host_frame_open = false;
        if self.pending_raf.is_empty() {
            self.raf_deadline = None;
            return;
        }
        if self.raf_deadline.is_none_or(|deadline| deadline <= now) {
            self.raf_deadline = Some(now + RAF_FRAME_INTERVAL);
        }
    }

    pub fn schedule_raf(&mut self, id: u64) {
        self.pending_raf.insert(id);
        if self.raf_deadline.is_none() {
            self.raf_deadline = Some(if self.host_frame_open {
                Instant::now() + RAF_FRAME_INTERVAL
            } else {
                Instant::now()
            });
        }
    }

    pub fn cancel_raf(&mut self, id: u64) {
        self.pending_raf.remove(&id);
        if self.pending_raf.is_empty() {
            self.raf_deadline = None;
        }
    }

    /// Due raf ids + timeout ids + interval ids that should fire.
    pub fn due_timers(&mut self, now: Instant) -> DueTimers {
        let raf = if self.raf_deadline.is_some_and(|deadline| deadline <= now) {
            self.raf_deadline = None;
            std::mem::take(&mut self.pending_raf).into_iter().collect()
        } else {
            Vec::new()
        };

        let mut timeouts = Vec::new();
        let ready: Vec<u64> = self
            .timeouts
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in ready {
            self.timeouts.remove(&id);
            timeouts.push(id);
        }

        let mut intervals = Vec::new();
        let mut reschedule = Vec::new();
        for (id, (at, period)) in &self.intervals {
            if *at <= now {
                intervals.push(*id);
                reschedule.push((*id, *period));
            }
        }
        for (id, period) in reschedule {
            self.intervals.insert(id, (now + period, period));
        }

        DueTimers {
            raf,
            timeouts,
            intervals,
        }
    }

    pub fn drain_fetch_completions(&mut self) -> Vec<HostValue> {
        self.fetch
            .drain_completions()
            .into_iter()
            .map(FetchCompletion::into_host_value)
            .collect()
    }

    pub fn drain_socket_events(&mut self) -> Vec<HostValue> {
        self.socket
            .drain_events()
            .into_iter()
            .map(SocketEvent::into_host_value)
            .collect()
    }

    /// Earliest useful host wakeup. Idle (no rAF, timer, fetch, or socket) is
    /// `None`. Pending rAF uses a stable deadline; an in-flight fetch or an
    /// open socket uses a short bounded wake until its next event arrives.
    pub fn next_wakeup(&self, now: Instant) -> Option<Instant> {
        let raf = self.raf_deadline;
        let timer = self
            .timeouts
            .values()
            .copied()
            .chain(self.intervals.values().map(|(at, _)| *at))
            .min();
        let fetch = self
            .fetch
            .has_pending()
            .then(|| now + Duration::from_millis(8));
        let socket = self
            .socket
            .has_active()
            .then(|| now + ws::SOCKET_WAKE_INTERVAL);
        raf.into_iter()
            .chain(timer)
            .chain(fetch)
            .chain(socket)
            .min()
    }
}

#[derive(Debug, Default, Clone)]
pub struct DueTimers {
    pub raf: Vec<u64>,
    pub timeouts: Vec<u64>,
    pub intervals: Vec<u64>,
}

impl DueTimers {
    pub fn is_empty(&self) -> bool {
        self.raf.is_empty() && self.timeouts.is_empty() && self.intervals.is_empty()
    }

    pub fn to_host_value(&self, now_ms: f64) -> HostValue {
        HostValue::Object(
            [
                (
                    "raf".into(),
                    HostValue::Array(
                        self.raf
                            .iter()
                            .map(|id| HostValue::Number(*id as f64))
                            .collect(),
                    ),
                ),
                (
                    "timeouts".into(),
                    HostValue::Array(
                        self.timeouts
                            .iter()
                            .map(|id| HostValue::Number(*id as f64))
                            .collect(),
                    ),
                ),
                (
                    "intervals".into(),
                    HostValue::Array(
                        self.intervals
                            .iter()
                            .map(|id| HostValue::Number(*id as f64))
                            .collect(),
                    ),
                ),
                ("now".into(), HostValue::Number(now_ms)),
            ]
            .into_iter()
            .collect(),
        )
    }
}

/// Shared handle used by VueHost and examples.
pub type SharedWebApiState = Arc<Mutex<WebApiState>>;
pub type SharedStorage = Arc<Mutex<BTreeMap<String, String>>>;

pub fn shared_storage() -> SharedStorage {
    Arc::new(Mutex::new(BTreeMap::new()))
}

pub fn shared_web_api_state() -> SharedWebApiState {
    Arc::new(Mutex::new(WebApiState::new()))
}

pub fn shared_web_api_state_with_fetch(fetch_host: SharedFetchHost) -> SharedWebApiState {
    Arc::new(Mutex::new(WebApiState::with_fetch_host(fetch_host)))
}

pub fn shared_web_api_state_with_local_storage(local_storage: SharedStorage) -> SharedWebApiState {
    Arc::new(Mutex::new(WebApiState::with_fetch_host_and_local_storage(
        shared_fetch_host(NativeFetchHost::new(FetchPolicy::default())),
        local_storage,
    )))
}

/// Register storage / timer / documentElement / location / clipboard host ops.
///
/// Clipboard defaults to the platform backend ([`default_shared_clipboard`]): OS
/// pasteboard on desktop, unsupported on Android.
pub fn register_web_api_host_ops(api: &mut HostApiRegistry, state: SharedWebApiState) {
    register_web_api_host_ops_with_resources(
        api,
        state,
        default_shared_clipboard(),
        shared_canvas_runtime(),
    );
}

/// Register the Web API surface with a caller-owned Canvas/image resource
/// store. VueHost uses this form so DOM canvas nodes and host rendering share
/// one lifecycle without coupling the generic web API crate to WGPU.
pub fn register_web_api_host_ops_with_resources(
    api: &mut HostApiRegistry,
    state: SharedWebApiState,
    clipboard: SharedClipboardHost,
    canvas: SharedCanvasRuntime,
) {
    register_web_api_storage_and_timer_ops(api, state);
    register_clipboard_host_ops(api, clipboard);
    canvas::register_canvas_host_ops(api, canvas);
    media::register_media_host_ops(api, shared_media_runtime());
}

/// Register media element / getUserMedia host ops against a caller-owned store.
pub fn register_media_host_ops(api: &mut HostApiRegistry, media: SharedMediaRuntime) {
    media::register_media_host_ops(api, media);
}

/// Like [`register_web_api_host_ops`], but with an injected clipboard (tests).
pub fn register_web_api_host_ops_with_clipboard(
    api: &mut HostApiRegistry,
    state: SharedWebApiState,
    clipboard: SharedClipboardHost,
) {
    register_web_api_host_ops_with_resources(api, state, clipboard, shared_canvas_runtime());
}

/// Register `clipboardReadText` / `clipboardWriteText` against a shared host.
pub fn register_clipboard_host_ops(api: &mut HostApiRegistry, clipboard: SharedClipboardHost) {
    {
        let clipboard = Arc::clone(&clipboard);
        api.register("clipboardReadText", move |_args| {
            let mut guard = clipboard
                .lock()
                .map_err(|_| JsException::new("clipboard state poisoned"))?;
            match guard.read_text() {
                Some(text) => Ok(HostValue::String(text)),
                None => Err(JsException::new("clipboard read failed")),
            }
        });
    }
    {
        let clipboard = Arc::clone(&clipboard);
        api.register("clipboardWriteText", move |args| {
            let text = arg_str(args, 0).unwrap_or_default();
            let mut guard = clipboard
                .lock()
                .map_err(|_| JsException::new("clipboard state poisoned"))?;
            if guard.write_text(&text) {
                Ok(HostValue::Null)
            } else {
                Err(JsException::new("clipboard write failed"))
            }
        });
    }
}

fn register_web_api_storage_and_timer_ops(api: &mut HostApiRegistry, state: SharedWebApiState) {
    fetch::register_fetch_host_ops(api, Arc::clone(&state));
    ws::register_socket_host_ops(api, Arc::clone(&state));
    {
        let state = Arc::clone(&state);
        api.register("storageGet", move |args| {
            let bucket = arg_str(args, 0).unwrap_or_else(|| "local".into());
            let key = arg_str(args, 1).unwrap_or_default();
            let guard = lock(&state)?;
            Ok(match guard.storage_get(&bucket, &key) {
                Some(v) => HostValue::String(v),
                None => HostValue::Null,
            })
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("storageSet", move |args| {
            let bucket = arg_str(args, 0).unwrap_or_else(|| "local".into());
            let key = arg_str(args, 1).unwrap_or_default();
            let value = arg_str(args, 2).unwrap_or_default();
            lock(&state)?.storage_set(&bucket, &key, value);
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("storageRemove", move |args| {
            let bucket = arg_str(args, 0).unwrap_or_else(|| "local".into());
            let key = arg_str(args, 1).unwrap_or_default();
            lock(&state)?.storage_remove(&bucket, &key);
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("storageClear", move |args| {
            let bucket = arg_str(args, 0).unwrap_or_else(|| "local".into());
            lock(&state)?.storage_clear(&bucket);
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("storageKeys", move |args| {
            let bucket = arg_str(args, 0).unwrap_or_else(|| "local".into());
            let keys = lock(&state)?.storage_keys(&bucket);
            Ok(HostValue::Array(
                keys.into_iter().map(HostValue::String).collect(),
            ))
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("rafSchedule", move |args| {
            let id = arg_u64(args, 0)?;
            lock(&state)?.schedule_raf(id);
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("rafCancel", move |args| {
            let id = arg_u64(args, 0)?;
            lock(&state)?.cancel_raf(id);
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("timeoutSchedule", move |args| {
            let id = arg_u64(args, 0)?;
            let ms = args
                .get(1)
                .and_then(HostValue::as_f64)
                .unwrap_or(0.0)
                .max(0.0);
            lock(&state)?
                .timeouts
                .insert(id, Instant::now() + Duration::from_millis(ms as u64));
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("timeoutCancel", move |args| {
            let id = arg_u64(args, 0)?;
            lock(&state)?.timeouts.remove(&id);
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("intervalSchedule", move |args| {
            let id = arg_u64(args, 0)?;
            let ms = args
                .get(1)
                .and_then(HostValue::as_f64)
                .unwrap_or(0.0)
                .max(0.0) as u64;
            let period = Duration::from_millis(ms);
            lock(&state)?
                .intervals
                .insert(id, (Instant::now() + period, period));
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("intervalCancel", move |args| {
            let id = arg_u64(args, 0)?;
            lock(&state)?.intervals.remove(&id);
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("documentElementSet", move |args| {
            let kind = arg_str(args, 0).unwrap_or_default();
            let key = arg_str(args, 1).unwrap_or_default();
            let value = args.get(2).cloned().unwrap_or(HostValue::Null);
            let mut guard = lock(&state)?;
            match kind.as_str() {
                "dataset" => match value {
                    HostValue::Null | HostValue::Undefined => {
                        guard.document_dataset.remove(&key);
                    }
                    other => {
                        guard.document_dataset.insert(key, host_to_string(&other));
                    }
                },
                "style" => match value {
                    HostValue::Null | HostValue::Undefined => {
                        guard.document_style.remove(&key);
                    }
                    other => {
                        guard.document_style.insert(key, host_to_string(&other));
                    }
                },
                _ => {}
            }
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("locationSet", move |args| {
            let path = arg_str(args, 0).unwrap_or_else(|| "/".into());
            let search = arg_str(args, 1).unwrap_or_default();
            let hash = arg_str(args, 2).unwrap_or_default();
            let mut guard = lock(&state)?;
            guard.location_path = path;
            guard.location_search = search;
            guard.location_hash = hash;
            Ok(HostValue::Null)
        });
    }
    {
        let state = Arc::clone(&state);
        api.register("webApiSnapshot", move |_args| {
            let guard = lock(&state)?;
            Ok(HostValue::Object(
                [
                    (
                        "path".into(),
                        HostValue::String(guard.location_path.clone()),
                    ),
                    (
                        "search".into(),
                        HostValue::String(guard.location_search.clone()),
                    ),
                    (
                        "theme".into(),
                        HostValue::String(
                            guard
                                .document_dataset
                                .get("theme")
                                .cloned()
                                .unwrap_or_default(),
                        ),
                    ),
                    (
                        "storageKeys".into(),
                        HostValue::Number(guard.storage_keys("local").len() as f64),
                    ),
                ]
                .into_iter()
                .collect(),
            ))
        });
    }
}

fn lock(state: &SharedWebApiState) -> Result<std::sync::MutexGuard<'_, WebApiState>, JsException> {
    state
        .lock()
        .map_err(|_| JsException::new("web-api state poisoned"))
}

fn arg_str(args: &[HostValue], index: usize) -> Option<String> {
    args.get(index).and_then(|v| match v {
        HostValue::String(s) => Some(s.clone()),
        HostValue::Number(n) => Some(n.to_string()),
        HostValue::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

fn arg_u64(args: &[HostValue], index: usize) -> Result<u64, JsException> {
    args.get(index)
        .and_then(HostValue::as_f64)
        .map(|n| n as u64)
        .ok_or_else(|| JsException::new(format!("expected number at arg {index}")))
}

fn host_to_string(value: &HostValue) -> String {
    match value {
        HostValue::Null | HostValue::Undefined => String::new(),
        HostValue::Bool(v) => v.to_string(),
        HostValue::Number(v) => {
            if v.fract() == 0.0 && v.is_finite() {
                format!("{}", *v as i64)
            } else {
                v.to_string()
            }
        }
        HostValue::String(v) => v.clone(),
        other => other.to_json_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_is_nonempty_and_mentions_window() {
        assert!(WEB_API_SHIM_JS.contains("localStorage"));
        assert!(WEB_API_SHIM_JS.contains("matchMedia"));
        assert!(WEB_API_SHIM_JS.contains("hostCall(\"evaluateMediaQuery\""));
        assert!(WEB_API_SHIM_JS.contains("evaluateMediaQueryLocal"));
        assert!(WEB_API_SHIM_JS.contains("ty === \"all\" || ty === \"screen\""));
        assert!(WEB_API_SHIM_JS.contains("requestAnimationFrame"));
        assert!(WEB_API_SHIM_JS.contains("ResizeObserver"));
        assert!(WEB_API_SHIM_JS.contains("history"));
        assert!(WEB_API_SHIM_JS.contains("querySelectorAll"));
        assert!(WEB_API_SHIM_JS.contains("getElementById"));
        assert!(WEB_API_SHIM_JS.contains("HTMLElement"));
        assert!(WEB_API_SHIM_JS.contains("parseHtmlFragment"));
        assert!(WEB_API_SHIM_JS.contains("createTemplateContent"));
        assert!(!WEB_API_SHIM_JS.contains("content stays empty"));
        assert!(WEB_API_SHIM_JS.contains("mediaDevices"));
        assert!(WEB_API_SHIM_JS.contains("mediaDevicesGetUserMedia"));
        assert!(WEB_API_SHIM_JS.contains("HTMLVideoElement"));
        assert!(WEB_API_SHIM_JS.contains("HTMLAudioElement"));
    }

    #[test]
    fn shim_clipboard_calls_host_ops_not_empty_resolve() {
        assert!(WEB_API_SHIM_JS.contains("clipboardWriteText"));
        assert!(WEB_API_SHIM_JS.contains("clipboardReadText"));
        assert!(WEB_API_SHIM_JS.contains("navigator"));
        // Must not keep the empty Promise.resolve stub without hostCall.
        let clip_idx = WEB_API_SHIM_JS
            .find("clipboard:")
            .expect("navigator.clipboard shim");
        let clip_snip = &WEB_API_SHIM_JS[clip_idx..clip_idx + 450];
        assert!(
            clip_snip.contains("hostCall(\"clipboardWriteText\""),
            "writeText must call host"
        );
        assert!(
            clip_snip.contains("hostCall(\"clipboardReadText\""),
            "readText must call host"
        );
    }

    #[test]
    fn memory_clipboard_roundtrip_via_host_ops() {
        let state = shared_web_api_state();
        let clipboard = shared_clipboard(MemoryClipboard::new());
        let mut api = HostApiRegistry::new();
        register_web_api_host_ops_with_clipboard(
            &mut api,
            Arc::clone(&state),
            Arc::clone(&clipboard),
        );

        api.call("clipboardWriteText", &[HostValue::string("nana-clipboard")])
            .expect("write ok");
        let got = api.call("clipboardReadText", &[]).expect("read ok");
        assert_eq!(got.as_str(), Some("nana-clipboard"));
    }

    #[test]
    fn unsupported_clipboard_host_ops_fail_honestly() {
        let state = shared_web_api_state();
        let clipboard = shared_clipboard(UnsupportedClipboard);
        let mut api = HostApiRegistry::new();
        register_web_api_host_ops_with_clipboard(&mut api, state, clipboard);

        let write_err = api
            .call("clipboardWriteText", &[HostValue::string("x")])
            .expect_err("unsupported write must fail");
        assert!(write_err.message.contains("clipboard write failed"));
        let read_err = api
            .call("clipboardReadText", &[])
            .expect_err("unsupported read must fail");
        assert!(read_err.message.contains("clipboard read failed"));
    }

    #[test]
    fn resize_observer_shim_reads_layout_box_not_fake_sidebar() {
        assert!(
            WEB_API_SHIM_JS.contains("__nanaNotifyLayout"),
            "layout pump hook must exist for ResizeObserver"
        );
        assert!(
            WEB_API_SHIM_JS.contains("layoutBox"),
            "ResizeObserver must query host layoutBox"
        );
        let start = WEB_API_SHIM_JS
            .find("ResizeObserver = function ResizeObserver")
            .expect("ResizeObserver ctor");
        let end = WEB_API_SHIM_JS[start..]
            .find("if (typeof globalThis.MutationObserver")
            .map(|i| start + i)
            .unwrap_or(WEB_API_SHIM_JS.len());
        let ro = &WEB_API_SHIM_JS[start..end];
        assert!(
            !ro.contains("220") && !ro.contains("640"),
            "ResizeObserver must not hardcode Navigation fake size 220×640"
        );
    }

    #[test]
    fn shim_projects_layout_box_onto_element_metrics() {
        assert!(WEB_API_SHIM_JS.contains("installElementLayoutMetrics"));
        assert!(WEB_API_SHIM_JS.contains("offsetWidth"));
        assert!(WEB_API_SHIM_JS.contains("offsetLeft"));
        assert!(WEB_API_SHIM_JS.contains("clientWidth"));
        assert!(WEB_API_SHIM_JS.contains("clientHeight"));
        assert!(WEB_API_SHIM_JS.contains("scrollWidth"));
        assert!(WEB_API_SHIM_JS.contains("layoutBox"));
    }

    #[test]
    fn shim_scroll_into_view_calls_host_op() {
        assert!(
            WEB_API_SHIM_JS.contains("hostCall(\"scrollIntoView\""),
            "scrollIntoView must call host scrollIntoView"
        );
        assert!(WEB_API_SHIM_JS.contains("getScrollOffset"));
        assert!(WEB_API_SHIM_JS.contains("setScrollOffset"));
        assert!(
            !WEB_API_SHIM_JS.contains("node.scrollIntoView = function () {};"),
            "empty scrollIntoView stub must be gone"
        );
    }

    #[test]
    fn shim_pumps_window_lifecycle_events() {
        assert!(
            WEB_API_SHIM_JS.contains("__nanaPumpLifecycle"),
            "host must pump focus/blur/resize/visibilitychange into shim EventTarget"
        );
        assert!(WEB_API_SHIM_JS.contains("type === \"focus\""));
        assert!(WEB_API_SHIM_JS.contains("type === \"blur\""));
        assert!(WEB_API_SHIM_JS.contains("type === \"resize\""));
        assert!(WEB_API_SHIM_JS.contains("type === \"visibilitychange\""));
        assert!(WEB_API_SHIM_JS.contains("__nanaFocused"));
        assert!(WEB_API_SHIM_JS.contains("visibilityState"));
    }

    #[test]
    fn storage_roundtrip_via_host_ops() {
        let state = shared_web_api_state();
        let mut api = HostApiRegistry::new();
        register_web_api_host_ops(&mut api, Arc::clone(&state));
        api.call(
            "storageSet",
            &[
                HostValue::string("local"),
                HostValue::string("k"),
                HostValue::string("v"),
            ],
        )
        .unwrap();
        let got = api
            .call(
                "storageGet",
                &[HostValue::string("local"), HostValue::string("k")],
            )
            .unwrap();
        assert_eq!(got.as_str(), Some("v"));
    }

    #[test]
    fn raf_and_timeout_become_due() {
        let state = shared_web_api_state();
        let mut api = HostApiRegistry::new();
        register_web_api_host_ops(&mut api, Arc::clone(&state));
        api.call("rafSchedule", &[HostValue::Number(1.0)]).unwrap();
        api.call(
            "timeoutSchedule",
            &[HostValue::Number(2.0), HostValue::Number(0.0)],
        )
        .unwrap();
        let due = state.lock().unwrap().due_timers(Instant::now());
        assert!(due.raf.contains(&1));
        assert!(due.timeouts.contains(&2));
    }

    #[test]
    fn raf_wakeup_deadline_is_stable_until_drained_or_cancelled() {
        let state = shared_web_api_state();
        let mut api = HostApiRegistry::new();
        register_web_api_host_ops(&mut api, Arc::clone(&state));
        api.call("rafSchedule", &[HostValue::Number(1.0)]).unwrap();
        let first = state
            .lock()
            .unwrap()
            .next_wakeup(Instant::now())
            .expect("rAF wakeup");
        std::thread::sleep(Duration::from_millis(1));
        let second = state
            .lock()
            .unwrap()
            .next_wakeup(Instant::now())
            .expect("same rAF wakeup");
        assert_eq!(first, second);

        api.call("rafCancel", &[HostValue::Number(1.0)]).unwrap();
        assert!(state.lock().unwrap().next_wakeup(Instant::now()).is_none());
    }

    #[test]
    fn nested_raf_requires_second_due_pass() {
        // Vue Transition nextFrame schedules rAF#2 inside rAF#1 callback.
        // Mimic a host frame: first due_timers clears pending; a nested
        // schedule waits for the next wakeup instead of spinning.
        let state = shared_web_api_state();
        let mut api = HostApiRegistry::new();
        register_web_api_host_ops(&mut api, Arc::clone(&state));
        api.call("rafSchedule", &[HostValue::Number(1.0)]).unwrap();
        let now = Instant::now();
        state.lock().unwrap().begin_host_frame(now);
        let due1 = state.lock().unwrap().due_timers(now);
        assert_eq!(due1.raf, vec![1]);
        assert!(state.lock().unwrap().due_timers(now).is_empty());
        api.call("rafSchedule", &[HostValue::Number(2.0)]).unwrap();
        assert!(
            state.lock().unwrap().due_timers(now).is_empty(),
            "nested rAF must not drain in the same host frame"
        );
        state.lock().unwrap().end_host_frame(now);
        let wakeup = state
            .lock()
            .unwrap()
            .next_wakeup(now)
            .expect("nested rAF wakeup");
        assert!(wakeup > now);
        let due2 = state.lock().unwrap().due_timers(wakeup);
        assert_eq!(due2.raf, vec![2]);
    }

    #[test]
    fn idle_next_wakeup_is_none() {
        let state = WebApiState::new();
        assert!(state.next_wakeup(Instant::now()).is_none());
    }

    #[test]
    fn shim_event_target_supports_capture_and_multi_listener() {
        assert!(
            WEB_API_SHIM_JS.contains("__nanaInvokePhase"),
            "EventTarget must expose phase invoke for Nana fan-out"
        );
        assert!(
            WEB_API_SHIM_JS.contains("normalizeListenerOptions"),
            "addEventListener must parse capture/once/passive options"
        );
        assert!(
            WEB_API_SHIM_JS.contains("entry.capture"),
            "listeners must retain capture flag for multi-listener dispatch"
        );
    }

    #[test]
    fn shim_websocket_uses_reserved_host_ops() {
        assert!(WEB_API_SHIM_JS.contains("globalThis.WebSocket = WebSocketShim"));
        assert!(WEB_API_SHIM_JS.contains("win.WebSocket = WebSocketShim"));
        assert!(WEB_API_SHIM_JS.contains("hostCall(\"wsOpen\""));
        assert!(WEB_API_SHIM_JS.contains("hostCall(\"wsSend\""));
        assert!(WEB_API_SHIM_JS.contains("hostCall(\"wsClose\""));
        assert!(WEB_API_SHIM_JS.contains("__nanaDrainWs"));
        assert!(
            WEB_API_SHIM_JS.contains("Nana WebSocket only supports ws:// and wss:// URLs"),
            "shim must reject non-WS schemes before the host op"
        );
    }

    #[test]
    fn compose_runtime_artifact_orders_shim_before_app() {
        let art = compose_runtime_artifact("app.js", "globalThis.__APP__=1;");
        let src = art.source_utf8().unwrap();
        assert!(src.contains("installNanaWebApiShim"));
        assert!(src.contains("__APP__=1"));
        assert!(src.find("installNanaWebApiShim").unwrap() < src.find("__APP__=1").unwrap());
    }
}
