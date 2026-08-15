//! Engine-agnostic JavaScript host interface for NanaUI Vue.
//!
//! Concrete QuickJS / V8 types must not leak through this crate. Applications and
//! `nana-ui-vue` depend only on these types; each app links exactly one of
//! `nana-js-quickjs` or `nana-js-v8`.

use std::any::Any;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// JS installed by each engine's host bridge.
///
/// Cycle-safe stringify so `__nanaHost.call` never feeds Vue vnode graphs /
/// DOM-likes / `on*` handlers into `JSON.stringify`.
pub const HOST_BRIDGE_INSTALL_JS: &str = r#"
globalThis.__nanaJsonStringify = function __nanaJsonStringify(value) {
  const seen = new WeakSet();
  function walk(v, depth) {
    if (typeof v === "function" || typeof v === "symbol" || typeof v === "undefined") {
      return undefined;
    }
    if (v === null || typeof v !== "object") return v;
    if (depth > 6) return undefined;
    if (typeof ArrayBuffer !== "undefined") {
      if (v instanceof ArrayBuffer) {
        return Array.from(new Uint8Array(v));
      }
      if (typeof ArrayBuffer.isView === "function" && ArrayBuffer.isView(v)) {
        return Array.from(new Uint8Array(v.buffer, v.byteOffset, v.byteLength));
      }
    }
    if (seen.has(v)) return undefined;
    if (typeof v.__nid === "number" || typeof v.nodeType === "number") {
      return undefined;
    }
    seen.add(v);
    if (Array.isArray(v)) {
      const out = [];
      for (let i = 0; i < v.length; i++) {
        out.push(walk(v[i], depth + 1));
      }
      return out;
    }
    const out = {};
    for (const k of Object.keys(v)) {
      if (k === "key" || k === "ref" || (k.charCodeAt(0) === 111 && k.charCodeAt(1) === 110)) {
        continue;
      }
      const sv = walk(v[k], depth + 1);
      if (typeof sv !== "undefined") out[k] = sv;
    }
    return out;
  }
  try {
    return JSON.stringify(walk(value, 0));
  } catch (_err) {
    return "[]";
  }
};
globalThis.__nanaHost = {
  call(name, args) {
    if (typeof globalThis.__nanaHostCallRaw === "function") {
      return globalThis.__nanaHostCallRaw(String(name), args ?? []);
    }
    const raw = globalThis.__nanaHostRaw(
      String(name),
      globalThis.__nanaJsonStringify(args ?? [])
    );
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object" && Object.prototype.hasOwnProperty.call(parsed, "__nanaHostError")) {
      throw new Error(String(parsed.__nanaHostError));
    }
    return parsed;
  },
  invoke(name, args, options) {
    if (typeof globalThis.__nanaHostInvokeRaw !== "function") {
      try {
        return Promise.resolve(this.call(name, args));
      } catch (error) {
        return Promise.reject(error);
      }
    }
    let requestId;
    try {
      requestId = String(globalThis.__nanaHostInvokeRaw(String(name), args ?? []));
    } catch (error) {
      return Promise.reject(error);
    }
    const opts = options && typeof options === "object" ? options : {};
    return new Promise((resolve, reject) => {
      let timer = null;
      let abort = null;
      const cleanup = () => {
        if (timer !== null && typeof clearTimeout === "function") clearTimeout(timer);
        if (abort && opts.signal && typeof opts.signal.removeEventListener === "function") {
          opts.signal.removeEventListener("abort", abort);
        }
      };
      globalThis.__nanaHostPending.set(requestId, { resolve, reject, cleanup });
      const cancel = (reason, name) => {
        const pending = globalThis.__nanaHostPending.get(requestId);
        if (!pending) return;
        globalThis.__nanaHostPending.delete(requestId);
        cleanup();
        globalThis.__nanaHostCancelRaw(requestId);
        const error = new Error(reason);
        error.name = name;
        reject(error);
      };
      if (opts.signal && typeof opts.signal.addEventListener === "function") {
        abort = () => cancel("host request aborted", "AbortError");
        if (opts.signal.aborted) {
          abort();
          return;
        }
        opts.signal.addEventListener("abort", abort, { once: true });
      }
      const timeout = Number(opts.timeout);
      if (Number.isFinite(timeout) && timeout >= 0 && typeof setTimeout === "function") {
        timer = setTimeout(() => cancel("host request timed out", "TimeoutError"), timeout);
      }
    });
  }
};
globalThis.__nanaHostPending = globalThis.__nanaHostPending || new Map();
globalThis.__nanaHostSettle = function __nanaHostSettle(requestId, ok, value) {
  const key = String(requestId);
  const pending = globalThis.__nanaHostPending.get(key);
  if (!pending) return false;
  globalThis.__nanaHostPending.delete(key);
  pending.cleanup();
  if (ok) {
    pending.resolve(value);
  } else {
    const message = value && typeof value === "object" && "message" in value
      ? String(value.message)
      : String(value);
    const error = new Error(message);
    if (value && typeof value === "object" && value.name) error.name = String(value.name);
    if (value && typeof value === "object" && value.code) error.code = String(value.code);
    if (value && typeof value === "object" && value.stack) error.stack = String(value.stack);
    if (value && typeof value === "object" && "details" in value) error.details = value.details;
    pending.reject(error);
  }
  return true;
};
globalThis.__nanaHostListeners = globalThis.__nanaHostListeners || new Map();
globalThis.__nanaHostEmit = function __nanaHostEmit(name, payload) {
  const listeners = globalThis.__nanaHostListeners.get(String(name));
  if (!listeners) return 0;
  for (const listener of Array.from(listeners)) listener(payload);
  return listeners.size;
};
globalThis.Nana = globalThis.Nana || {};
globalThis.Nana.host = {
  call: globalThis.__nanaHost.call.bind(globalThis.__nanaHost),
  invoke: globalThis.__nanaHost.invoke.bind(globalThis.__nanaHost),
  on(name, listener) {
    if (typeof listener !== "function") throw new TypeError("host event listener must be a function");
    const key = String(name);
    let listeners = globalThis.__nanaHostListeners.get(key);
    if (!listeners) {
      listeners = new Set();
      globalThis.__nanaHostListeners.set(key, listeners);
    }
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) globalThis.__nanaHostListeners.delete(key);
    };
  }
};
globalThis.Nana.resources = {
  release(handle) {
    if (typeof globalThis.__nanaHostReleaseResourceRaw !== "function") return false;
    return Boolean(globalThis.__nanaHostReleaseResourceRaw(handle));
  }
};
"#;

/// Opaque handle for a JS function retained by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JsFunctionId(pub u64);

/// Opaque handle for a JS object retained by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JsObjectId(pub u64);

/// Opaque, generation-checked handle for data owned outside V8.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostResourceHandle {
    pub id: u64,
    pub generation: u32,
    pub kind: String,
}

impl HostResourceHandle {
    pub fn new(id: u64, generation: u32, kind: impl Into<String>) -> Self {
        Self {
            id,
            generation,
            kind: kind.into(),
        }
    }
}

/// Weak reference placeholder for host DOM/node handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JsWeakRef(pub u64);

/// Module specifier string used when loading ES modules.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleSpecifier(pub String);

impl ModuleSpecifier {
    pub fn new(spec: impl Into<String>) -> Self {
        Self(spec.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Engine-neutral value exchanged across the Rust ↔ JS boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum HostValue {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    /// Unsigned 64-bit integer. V8 maps this to a JavaScript `bigint` so
    /// resource identities are never truncated through IEEE-754 numbers.
    BigInt(u64),
    String(String),
    /// Raw bytes. V8 maps this to an `ArrayBuffer` without JSON/Base64 encoding.
    Bytes(Vec<u8>),
    Array(Vec<HostValue>),
    Object(BTreeMap<String, HostValue>),
    /// Opaque host-owned resource exposed to JavaScript as a branded handle.
    Resource(HostResourceHandle),
    Function(JsFunctionId),
    ObjectRef(JsObjectId),
}

impl HostValue {
    pub fn string(s: impl Into<String>) -> Self {
        Self::String(s.into())
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::BigInt(value) => Some(*value),
            Self::Number(value)
                if value.is_finite()
                    && *value >= 0.0
                    && value.fract() == 0.0
                    && *value <= u64::MAX as f64 =>
            {
                Some(*value as u64)
            }
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub fn as_resource(&self) -> Option<&HostResourceHandle> {
        match self {
            Self::Resource(handle) => Some(handle),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, HostValue>> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<HostValue>> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// Encode a JSON-compatible subset for engine host bridges.
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            Self::Null | Self::Undefined => serde_json::Value::Null,
            Self::Bool(v) => serde_json::Value::Bool(*v),
            Self::Number(v) => serde_json::Number::from_f64(*v)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Self::BigInt(v) => serde_json::json!({ "__bigint": v.to_string() }),
            Self::String(v) => serde_json::Value::String(v.clone()),
            Self::Bytes(bytes) => serde_json::json!({ "__bytes": bytes }),
            Self::Array(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json_value).collect())
            }
            Self::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), v.to_json_value()))
                    .collect(),
            ),
            Self::Resource(handle) => serde_json::json!({
                "__resource": true,
                "id": handle.id.to_string(),
                "generation": handle.generation,
                "kind": handle.kind,
            }),
            Self::Function(id) => serde_json::json!({ "__fn": id.0 }),
            Self::ObjectRef(id) => serde_json::json!({ "__obj": id.0 }),
        }
    }

    pub fn to_json_string(&self) -> String {
        self.to_json_value().to_string()
    }

    pub fn from_json_value(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(v) => Self::Bool(v),
            serde_json::Value::Number(v) => Self::Number(v.as_f64().unwrap_or(f64::NAN)),
            serde_json::Value::String(v) => Self::String(v),
            serde_json::Value::Array(items) => {
                Self::Array(items.into_iter().map(Self::from_json_value).collect())
            }
            serde_json::Value::Object(map) => {
                if let Some(value) = map
                    .get("__bigint")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| value.parse().ok())
                {
                    return Self::BigInt(value);
                }
                if let Some(bytes) = map.get("__bytes").and_then(serde_json::Value::as_array) {
                    let bytes = bytes
                        .iter()
                        .filter_map(serde_json::Value::as_u64)
                        .map(|byte| byte as u8)
                        .collect();
                    return Self::Bytes(bytes);
                }
                if map.get("__resource").and_then(serde_json::Value::as_bool) == Some(true) {
                    let id = map
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|id| id.parse().ok())
                        .or_else(|| map.get("id").and_then(serde_json::Value::as_u64));
                    let generation = map
                        .get("generation")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|generation| generation.try_into().ok());
                    let kind = map.get("kind").and_then(serde_json::Value::as_str);
                    if let (Some(id), Some(generation), Some(kind)) = (id, generation, kind) {
                        return Self::Resource(HostResourceHandle::new(id, generation, kind));
                    }
                }
                if let Some(id) = map.get("__fn").and_then(serde_json::Value::as_u64) {
                    return Self::Function(JsFunctionId(id));
                }
                if let Some(id) = map.get("__obj").and_then(serde_json::Value::as_u64) {
                    return Self::ObjectRef(JsObjectId(id));
                }
                Self::Object(
                    map.into_iter()
                        .map(|(k, v)| (k, Self::from_json_value(v)))
                        .collect(),
                )
            }
        }
    }

    pub fn from_json_str(s: &str) -> Result<Self, JsEngineError> {
        let value = serde_json::from_str(s)
            .map_err(|err| JsEngineError::new(format!("invalid host JSON: {err}")))?;
        Ok(Self::from_json_value(value))
    }
}

/// Kind of bytes carried by [`RuntimeArtifact`].
///
/// QuickJS bytecode and V8 snapshots are **not** interchangeable. Dev / dual-engine
/// paths use [`RuntimeArtifactKind::SourceUtf8`]; Release embeds one engine-native
/// binary form so business JS is not shipped as plaintext.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeArtifactKind {
    /// UTF-8 JavaScript source (shared by QuickJS and V8 in development).
    SourceUtf8,
    /// QuickJS-NG module bytecode from `JS_WriteObject` / `Module::write`.
    QuickJsBytecode,
    /// V8 `StartupData` snapshot blob from `SnapshotCreator::create_blob`.
    V8Snapshot,
}

/// Compiled or source runtime artifact selected by the active engine.
#[derive(Debug, Clone)]
pub struct RuntimeArtifact {
    pub bytes: Vec<u8>,
    pub name: String,
    pub kind: RuntimeArtifactKind,
}

impl RuntimeArtifact {
    pub fn from_source(name: impl Into<String>, source: impl AsRef<[u8]>) -> Self {
        Self {
            bytes: source.as_ref().to_vec(),
            name: name.into(),
            kind: RuntimeArtifactKind::SourceUtf8,
        }
    }

    pub fn from_quickjs_bytecode(name: impl Into<String>, bytes: impl AsRef<[u8]>) -> Self {
        Self {
            bytes: bytes.as_ref().to_vec(),
            name: name.into(),
            kind: RuntimeArtifactKind::QuickJsBytecode,
        }
    }

    pub fn from_v8_snapshot(name: impl Into<String>, bytes: impl AsRef<[u8]>) -> Self {
        Self {
            bytes: bytes.as_ref().to_vec(),
            name: name.into(),
            kind: RuntimeArtifactKind::V8Snapshot,
        }
    }

    pub fn source_utf8(&self) -> Result<&str, JsEngineError> {
        if self.kind != RuntimeArtifactKind::SourceUtf8 {
            return Err(JsEngineError::new(format!(
                "runtime artifact `{}` is {:?}, not UTF-8 source",
                self.name, self.kind
            )));
        }
        std::str::from_utf8(&self.bytes).map_err(|err| JsEngineError {
            message: format!("runtime artifact is not UTF-8: {err}"),
            exception: None,
        })
    }

    pub fn is_binary_release(&self) -> bool {
        matches!(
            self.kind,
            RuntimeArtifactKind::QuickJsBytecode | RuntimeArtifactKind::V8Snapshot
        )
    }
}

/// JavaScript exception captured at the host boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct JsException {
    pub name: String,
    pub code: Option<String>,
    pub message: String,
    pub stack: Option<String>,
    pub details: Option<HostValue>,
}

impl JsException {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            name: "Error".into(),
            code: None,
            message: message.into(),
            stack: None,
            details: None,
        }
    }

    pub fn with_stack(message: impl Into<String>, stack: impl Into<String>) -> Self {
        Self {
            name: "Error".into(),
            code: None,
            message: message.into(),
            stack: Some(stack.into()),
            details: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_details(mut self, details: HostValue) -> Self {
        self.details = Some(details);
        self
    }

    pub fn to_host_value(&self) -> HostValue {
        let mut value = BTreeMap::from([
            ("name".into(), HostValue::string(&self.name)),
            ("message".into(), HostValue::string(&self.message)),
        ]);
        if let Some(code) = &self.code {
            value.insert("code".into(), HostValue::string(code));
        }
        if let Some(stack) = &self.stack {
            value.insert("stack".into(), HostValue::string(stack));
        }
        if let Some(details) = &self.details {
            value.insert("details".into(), details.clone());
        }
        HostValue::Object(value)
    }
}

impl fmt::Display for JsException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsException {}

/// Runtime diagnostic emitted by a JS backend or the Vue compatibility layer.
/// It contains no product data unless the caller explicitly puts it in the
/// message, and is intended for development tooling rather than product UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsDiagnosticEvent {
    pub source: String,
    pub level: JsDiagnosticLevel,
    pub message: String,
    pub stack: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsDiagnosticLevel {
    Info,
    Warning,
    Error,
}

pub type JsDiagnosticSink = Arc<dyn Fn(JsDiagnosticEvent) + Send + Sync>;

/// Privacy-preserving Host API timing. Arguments and return values are not
/// captured because they may contain application or user data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCallTrace {
    pub name: String,
    pub asynchronous: bool,
    pub pending: bool,
    pub succeeded: bool,
    pub duration_micros: u64,
}

pub type HostCallObserver = Arc<dyn Fn(HostCallTrace) + Send + Sync>;

/// Engine-level failure (init, invoke, conversion, or JS exception).
#[derive(Debug, Clone, PartialEq)]
pub struct JsEngineError {
    pub message: String,
    pub exception: Option<JsException>,
}

impl JsEngineError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exception: None,
        }
    }

    pub fn from_exception(exception: JsException) -> Self {
        Self {
            message: exception.message.clone(),
            exception: Some(exception),
        }
    }
}

impl fmt::Display for JsEngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsEngineError {}

#[derive(Default)]
struct HostResourceState {
    next_id: u64,
    free_ids: Vec<u64>,
    generations: BTreeMap<u64, u32>,
    entries: BTreeMap<u64, HostResourceEntry>,
}

struct HostResourceEntry {
    generation: u32,
    kind: String,
    value: Arc<dyn Any + Send + Sync>,
}

/// Context-owned storage behind [`HostResourceHandle`].
///
/// Released IDs may be reused, but their generation is incremented so stale JS
/// handles cannot access a replacement resource.
#[derive(Clone, Default)]
pub struct HostResourceRegistry {
    state: Arc<Mutex<HostResourceState>>,
}

impl HostResourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T>(&self, kind: impl Into<String>, value: T) -> HostResourceHandle
    where
        T: Any + Send + Sync + 'static,
    {
        self.insert_arc(kind, Arc::new(value))
    }

    pub fn insert_arc<T>(&self, kind: impl Into<String>, value: Arc<T>) -> HostResourceHandle
    where
        T: Any + Send + Sync + 'static,
    {
        let mut state = self.state.lock().expect("host resource registry");
        let id = state.free_ids.pop().unwrap_or_else(|| {
            state.next_id = state.next_id.saturating_add(1).max(1);
            state.next_id
        });
        let generation = state
            .generations
            .get(&id)
            .copied()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);
        state.generations.insert(id, generation);
        let kind = kind.into();
        state.entries.insert(
            id,
            HostResourceEntry {
                generation,
                kind: kind.clone(),
                value,
            },
        );
        HostResourceHandle::new(id, generation, kind)
    }

    pub fn get<T>(&self, handle: &HostResourceHandle) -> Option<Arc<T>>
    where
        T: Any + Send + Sync + 'static,
    {
        let state = self.state.lock().ok()?;
        let entry = state.entries.get(&handle.id)?;
        if entry.generation != handle.generation || entry.kind != handle.kind {
            return None;
        }
        Arc::clone(&entry.value).downcast::<T>().ok()
    }

    pub fn contains(&self, handle: &HostResourceHandle) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        state
            .entries
            .get(&handle.id)
            .is_some_and(|entry| entry.generation == handle.generation && entry.kind == handle.kind)
    }

    pub fn release(&self, handle: &HostResourceHandle) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let matches = state.entries.get(&handle.id).is_some_and(|entry| {
            entry.generation == handle.generation && entry.kind == handle.kind
        });
        if !matches {
            return false;
        }
        state.entries.remove(&handle.id);
        state.free_ids.push(handle.id);
        true
    }

    pub fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            let released = state.entries.keys().copied().collect::<Vec<_>>();
            state.entries.clear();
            state.free_ids.extend(released);
        }
    }

    pub fn len(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.entries.len())
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for HostResourceRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostResourceRegistry")
            .field("len", &self.len())
            .finish()
    }
}

/// Monotonic identifier for one Promise-backed host invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostRequestId(pub u64);

/// Cooperative cancellation shared with asynchronous host work.
#[derive(Debug, Clone, Default)]
pub struct HostCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl HostCancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Context passed to an asynchronous host handler.
#[derive(Debug, Clone)]
pub struct HostRequestContext {
    pub id: HostRequestId,
    cancellation: HostCancellationToken,
    resources: HostResourceRegistry,
}

impl HostRequestContext {
    pub fn new(
        id: HostRequestId,
        cancellation: HostCancellationToken,
        resources: HostResourceRegistry,
    ) -> Self {
        Self {
            id,
            cancellation,
            resources,
        }
    }

    pub fn cancellation(&self) -> &HostCancellationToken {
        &self.cancellation
    }

    pub fn resources(&self) -> &HostResourceRegistry {
        &self.resources
    }

    pub fn pending(&self) -> (HostCompletion, HostPendingCall) {
        HostPendingCall::channel(self.cancellation.clone())
    }
}

/// One-shot completion endpoint that may be moved to another thread or executor.
pub struct HostCompletion {
    sender: Option<mpsc::Sender<Result<HostValue, JsException>>>,
    cancellation: HostCancellationToken,
}

impl HostCompletion {
    pub fn complete(mut self, result: Result<HostValue, JsException>) -> bool {
        if self.cancellation.is_cancelled() {
            self.sender.take();
            return false;
        }
        self.sender
            .take()
            .is_some_and(|sender| sender.send(result).is_ok())
    }

    pub fn resolve(self, value: HostValue) -> bool {
        self.complete(Ok(value))
    }

    pub fn reject(self, exception: JsException) -> bool {
        self.complete(Err(exception))
    }
}

/// Pending asynchronous work polled by a concrete JS engine.
pub struct HostPendingCall {
    receiver: Mutex<Receiver<Result<HostValue, JsException>>>,
    cancellation: HostCancellationToken,
}

impl HostPendingCall {
    pub fn channel(cancellation: HostCancellationToken) -> (HostCompletion, Self) {
        let (sender, receiver) = mpsc::channel();
        (
            HostCompletion {
                sender: Some(sender),
                cancellation: cancellation.clone(),
            },
            Self {
                receiver: Mutex::new(receiver),
                cancellation,
            },
        )
    }

    pub fn try_take(&self) -> Option<Result<HostValue, JsException>> {
        let receiver = self.receiver.lock().ok()?;
        match receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(JsException::new(
                "host request completion channel disconnected",
            ))),
        }
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

impl fmt::Debug for HostPendingCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostPendingCall")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Result of starting a host invocation.
#[derive(Debug)]
pub enum HostInvocation {
    Ready(Result<HostValue, JsException>),
    Pending(HostPendingCall),
}

/// One host-to-JavaScript event queued for the owning engine thread.
#[derive(Debug, Clone, PartialEq)]
pub struct HostEvent {
    pub name: String,
    pub payload: HostValue,
}

/// Thread-safe event sender. Concrete engines drain it on their normal pump.
#[derive(Debug, Clone)]
pub struct HostEventSender {
    queue: Arc<Mutex<HostEventQueue>>,
}

#[derive(Debug)]
struct HostEventQueue {
    events: VecDeque<QueuedHostEvent>,
    capacity: usize,
}

#[derive(Debug)]
struct QueuedHostEvent {
    event: HostEvent,
    reliable: bool,
}

impl Default for HostEventSender {
    fn default() -> Self {
        Self::with_capacity(4096)
    }
}

impl HostEventSender {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: Arc::new(Mutex::new(HostEventQueue {
                events: VecDeque::new(),
                capacity: capacity.max(1),
            })),
        }
    }

    pub fn send(&self, name: impl Into<String>, payload: HostValue) {
        let _ = self.try_send(name, payload);
    }

    /// Enqueue with explicit backpressure. A full queue preserves earlier
    /// ordered events, except that an already queued event with the same name
    /// is replaced by its newest state payload.
    pub fn try_send(&self, name: impl Into<String>, payload: HostValue) -> Result<(), HostEvent> {
        let event = HostEvent {
            name: name.into(),
            payload,
        };
        let Ok(mut queue) = self.queue.lock() else {
            return Err(event);
        };
        if queue.events.len() >= queue.capacity {
            if let Some(existing) = queue
                .events
                .iter_mut()
                .rev()
                .find(|existing| !existing.reliable && existing.event.name == event.name)
            {
                existing.event = event;
                return Ok(());
            }
            return Err(event);
        }
        queue.events.push_back(QueuedHostEvent {
            event,
            reliable: false,
        });
        Ok(())
    }

    /// Enqueue an ordered lifecycle/error event. When the queue is full it may
    /// evict one lossy state event, but never overwrites another reliable event.
    pub fn send_reliable(
        &self,
        name: impl Into<String>,
        payload: HostValue,
    ) -> Result<(), HostEvent> {
        let event = HostEvent {
            name: name.into(),
            payload,
        };
        let Ok(mut queue) = self.queue.lock() else {
            return Err(event);
        };
        if queue.events.len() >= queue.capacity {
            let Some(index) = queue.events.iter().position(|queued| !queued.reliable) else {
                return Err(event);
            };
            queue.events.remove(index);
        }
        queue.events.push_back(QueuedHostEvent {
            event,
            reliable: true,
        });
        Ok(())
    }

    /// Drain queued events. This is intended for concrete engine implementations.
    pub fn drain(&self) -> Vec<HostEvent> {
        let Ok(mut queue) = self.queue.lock() else {
            return Vec::new();
        };
        queue.events.drain(..).map(|queued| queued.event).collect()
    }

    pub fn clear(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.events.clear();
        }
    }

    pub fn len(&self) -> usize {
        self.queue
            .lock()
            .map(|queue| queue.events.len())
            .unwrap_or_default()
    }
}

/// Host API callback invoked from JavaScript via `__nanaHost.call(name, args)`.
pub type HostApiHandler = Arc<dyn Fn(&[HostValue]) -> Result<HostValue, JsException> + Send + Sync>;

/// Promise-backed host API callback invoked by `Nana.host.invoke`.
pub type HostAsyncApiHandler = Arc<
    dyn Fn(Vec<HostValue>, HostRequestContext) -> Result<HostPendingCall, JsException>
        + Send
        + Sync,
>;

/// Registry of named host callbacks shared by QuickJS and V8 backends.
#[derive(Default, Clone)]
pub struct HostApiRegistry {
    handlers: BTreeMap<String, HostApiHandler>,
    async_handlers: BTreeMap<String, HostAsyncApiHandler>,
    observer: Option<HostCallObserver>,
}

impl HostApiRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_observer(&mut self, observer: Option<HostCallObserver>) -> &mut Self {
        self.observer = observer;
        self
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        handler: impl Fn(&[HostValue]) -> Result<HostValue, JsException> + Send + Sync + 'static,
    ) -> &mut Self {
        let name = name.into();
        self.async_handlers.remove(&name);
        self.handlers.insert(name, Arc::new(handler));
        self
    }

    pub fn register_arc(&mut self, name: impl Into<String>, handler: HostApiHandler) -> &mut Self {
        let name = name.into();
        self.async_handlers.remove(&name);
        self.handlers.insert(name, handler);
        self
    }

    pub fn register_async(
        &mut self,
        name: impl Into<String>,
        handler: impl Fn(Vec<HostValue>, HostRequestContext) -> Result<HostPendingCall, JsException>
        + Send
        + Sync
        + 'static,
    ) -> &mut Self {
        let name = name.into();
        self.handlers.remove(&name);
        self.async_handlers.insert(name, Arc::new(handler));
        self
    }

    pub fn register_async_arc(
        &mut self,
        name: impl Into<String>,
        handler: HostAsyncApiHandler,
    ) -> &mut Self {
        let name = name.into();
        self.handlers.remove(&name);
        self.async_handlers.insert(name, handler);
        self
    }

    /// Add every handler from `additional` without allowing either registry to
    /// replace an existing name.
    ///
    /// The preflight keeps the operation atomic: on conflict, `self` is left
    /// unchanged. This is the boundary used by framework-owned and
    /// application-owned host APIs.
    pub fn try_extend(&mut self, additional: &Self) -> Result<&mut Self, JsEngineError> {
        let self_contains = |name: &String| {
            self.handlers.contains_key(name) || self.async_handlers.contains_key(name)
        };
        let conflict = additional
            .handlers
            .keys()
            .chain(additional.async_handlers.keys())
            .find(|name| self_contains(name));
        if let Some(name) = conflict {
            return Err(JsEngineError::new(format!(
                "duplicate host API name `{name}`"
            )));
        }
        self.handlers.extend(
            additional
                .handlers
                .iter()
                .map(|(name, handler)| (name.clone(), Arc::clone(handler))),
        );
        self.async_handlers.extend(
            additional
                .async_handlers
                .iter()
                .map(|(name, handler)| (name.clone(), Arc::clone(handler))),
        );
        Ok(self)
    }

    pub fn get(&self, name: &str) -> Option<&HostApiHandler> {
        self.handlers.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.handlers
            .keys()
            .chain(self.async_handlers.keys())
            .map(String::as_str)
    }

    pub fn call(&self, name: &str, args: &[HostValue]) -> Result<HostValue, JsException> {
        if self.async_handlers.contains_key(name) {
            return Err(JsException::new(format!(
                "host API `{name}` is asynchronous; use Nana.host.invoke"
            )));
        }
        let handler = self
            .handlers
            .get(name)
            .ok_or_else(|| JsException::new(format!("unknown host API `{name}`")))?;
        let started = Instant::now();
        let result = handler(args);
        self.observe(HostCallTrace {
            name: name.to_owned(),
            asynchronous: false,
            pending: false,
            succeeded: result.is_ok(),
            duration_micros: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        });
        result
    }

    pub fn invoke(
        &self,
        name: &str,
        args: Vec<HostValue>,
        context: HostRequestContext,
    ) -> HostInvocation {
        if let Some(handler) = self.async_handlers.get(name) {
            let started = Instant::now();
            let invocation = match handler(args, context) {
                Ok(pending) => HostInvocation::Pending(pending),
                Err(exception) => HostInvocation::Ready(Err(exception)),
            };
            self.observe(HostCallTrace {
                name: name.to_owned(),
                asynchronous: true,
                pending: matches!(invocation, HostInvocation::Pending(_)),
                succeeded: !matches!(invocation, HostInvocation::Ready(Err(_))),
                duration_micros: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            });
            return invocation;
        }
        HostInvocation::Ready(self.call(name, &args))
    }

    fn observe(&self, trace: HostCallTrace) {
        if let Some(observer) = &self.observer {
            observer(trace);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty() && self.async_handlers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.handlers.len() + self.async_handlers.len()
    }
}

impl fmt::Debug for HostApiRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostApiRegistry")
            .field("sync_handlers", &self.handlers.keys().collect::<Vec<_>>())
            .field(
                "async_handlers",
                &self.async_handlers.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Unified JS engine contract used by `nana-ui-vue`.
///
/// Paint and retained UI stay in Runtime/UiScene; this trait is JS execution only.
/// Iced is the current compatibility view, not part of this interface.
pub trait JsEngine {
    /// Evaluate / load a runtime artifact (UTF-8 JS source).
    fn initialize(&mut self, artifact: RuntimeArtifact) -> Result<(), JsEngineError>;

    /// Install host callbacks as `globalThis.__nanaHost.call(name, args)`.
    ///
    /// Prefer calling this before [`initialize`](Self::initialize) so the artifact
    /// can use host APIs during top-level evaluation.
    fn register_host_api(&mut self, api: &HostApiRegistry) -> Result<(), JsEngineError>;

    /// Resolve a global property path (for example `__nanaProbe.run`) to a function id.
    fn resolve_function(&mut self, name: &str) -> Result<JsFunctionId, JsEngineError>;

    /// Invoke a previously resolved JS function.
    fn invoke(
        &mut self,
        target: JsFunctionId,
        args: &[HostValue],
    ) -> Result<HostValue, JsEngineError>;

    /// Drain engine microtask / job queues (Vue `nextTick`, Promises, etc.).
    fn run_microtasks(&mut self) -> Result<(), JsEngineError>;

    /// Return a thread-safe sender for host-to-JS events when supported.
    fn host_event_sender(&self) -> Option<HostEventSender> {
        None
    }

    /// Context-owned resources exposed through host handles when supported.
    fn host_resources(&self) -> Option<HostResourceRegistry> {
        None
    }

    fn interrupt(&mut self);
    fn request_gc(&mut self);
    fn shutdown(&mut self);
}

/// Shared Phase 2 Vue `runtime-core` probe artifact (IIFE, UTF-8).
pub mod probe {
    use super::{HostApiRegistry, HostValue, JsException, RuntimeArtifact};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// Pre-bundled `@vue/runtime-core` probe used by both engines (stub host ops).
    pub const VUE_RUNTIME_PROBE_JS: &str =
        include_str!("../fixtures/vue-runtime-probe/dist/vue-runtime-probe.iife.js");

    /// Counter/Todo custom-renderer artifact — hostOps return Rust node handles.
    pub const VUE_PHASE3_JS: &str =
        include_str!("../fixtures/vue-runtime-probe/dist/vue-phase3.iife.js");

    /// Reproducible Vite-built Vue SFC + TypeScript compatibility artifact.
    pub const VUE_SFC_COMPAT_JS: &str =
        include_str!("../fixtures/vue-sfc-compat/dist/vue-sfc-compat.iife.js");
    pub const VUE_SFC_COMPAT_CSS: &str =
        include_str!("../fixtures/vue-sfc-compat/dist/nanaui-vue-sfc-compat-fixture.css");

    pub fn vue_runtime_probe_artifact() -> RuntimeArtifact {
        RuntimeArtifact::from_source("vue-runtime-probe.iife.js", VUE_RUNTIME_PROBE_JS)
    }

    pub fn vue_phase3_artifact() -> RuntimeArtifact {
        RuntimeArtifact::from_source("vue-phase3.iife.js", VUE_PHASE3_JS)
    }

    pub fn vue_sfc_compat_artifact() -> RuntimeArtifact {
        RuntimeArtifact::from_source("vue-sfc-compat.iife.js", VUE_SFC_COMPAT_JS)
    }

    /// Host-side counters recorded by renderer stub ops + probe callbacks.
    #[derive(Debug, Default, Clone, PartialEq, Eq)]
    pub struct ProbeHostState {
        pub create_element: u64,
        pub create_text: u64,
        pub insert: u64,
        pub remove: u64,
        pub patch_prop: u64,
        pub set_text: u64,
        pub set_element_text: u64,
        pub reactive_set: u64,
        pub increment: u64,
        pub last_count: i64,
    }

    impl ProbeHostState {
        pub fn to_host_value(&self) -> HostValue {
            let mut map = BTreeMap::new();
            map.insert(
                "createElement".into(),
                HostValue::Number(self.create_element as f64),
            );
            map.insert(
                "createText".into(),
                HostValue::Number(self.create_text as f64),
            );
            map.insert("insert".into(), HostValue::Number(self.insert as f64));
            map.insert("remove".into(), HostValue::Number(self.remove as f64));
            map.insert(
                "patchProp".into(),
                HostValue::Number(self.patch_prop as f64),
            );
            map.insert("setText".into(), HostValue::Number(self.set_text as f64));
            map.insert(
                "setElementText".into(),
                HostValue::Number(self.set_element_text as f64),
            );
            map.insert(
                "reactiveSet".into(),
                HostValue::Number(self.reactive_set as f64),
            );
            map.insert("increment".into(), HostValue::Number(self.increment as f64));
            map.insert(
                "lastCount".into(),
                HostValue::Number(self.last_count as f64),
            );
            HostValue::Object(map)
        }
    }

    /// Build the shared HostApiRegistry used by QuickJS and V8 probe runs.
    pub fn probe_host_registry() -> (HostApiRegistry, Arc<Mutex<ProbeHostState>>) {
        let state = Arc::new(Mutex::new(ProbeHostState::default()));
        let mut api = HostApiRegistry::new();

        let bump = |state: &Arc<Mutex<ProbeHostState>>, field: fn(&mut ProbeHostState)| {
            let state = Arc::clone(state);
            Arc::new(
                move |_args: &[HostValue]| -> Result<HostValue, JsException> {
                    let mut guard = state
                        .lock()
                        .map_err(|_| JsException::new("probe host state poisoned"))?;
                    field(&mut guard);
                    Ok(HostValue::Null)
                },
            ) as super::HostApiHandler
        };

        api.register_arc("createElement", bump(&state, |s| s.create_element += 1));
        api.register_arc("createText", bump(&state, |s| s.create_text += 1));
        api.register_arc("insert", bump(&state, |s| s.insert += 1));
        api.register_arc("remove", bump(&state, |s| s.remove += 1));
        api.register_arc("patchProp", bump(&state, |s| s.patch_prop += 1));
        api.register_arc("setText", bump(&state, |s| s.set_text += 1));
        api.register_arc("setElementText", bump(&state, |s| s.set_element_text += 1));

        {
            let state = Arc::clone(&state);
            api.register_arc(
                "reactiveSet",
                Arc::new(move |args: &[HostValue]| {
                    let mut guard = state
                        .lock()
                        .map_err(|_| JsException::new("probe host state poisoned"))?;
                    guard.reactive_set += 1;
                    if let Some(n) = args.first().and_then(HostValue::as_f64) {
                        guard.last_count = n as i64;
                    }
                    Ok(HostValue::Null)
                }),
            );
        }
        {
            let state = Arc::clone(&state);
            api.register_arc(
                "increment",
                Arc::new(move |args: &[HostValue]| {
                    let mut guard = state
                        .lock()
                        .map_err(|_| JsException::new("probe host state poisoned"))?;
                    guard.increment += 1;
                    if let Some(n) = args.first().and_then(HostValue::as_f64) {
                        guard.last_count = n as i64;
                    }
                    Ok(HostValue::Null)
                }),
            );
        }
        {
            let state = Arc::clone(&state);
            api.register_arc(
                "snapshot",
                Arc::new(move |_args: &[HostValue]| {
                    let guard = state
                        .lock()
                        .map_err(|_| JsException::new("probe host state poisoned"))?;
                    Ok(guard.to_host_value())
                }),
            );
        }

        (api, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{probe_host_registry, vue_runtime_probe_artifact};

    #[test]
    fn host_value_roundtrip_shapes() {
        let mut map = BTreeMap::new();
        map.insert("ok".into(), HostValue::Bool(true));
        let value = HostValue::Array(vec![
            HostValue::Null,
            HostValue::Number(2.0),
            HostValue::string("x"),
            HostValue::Object(map),
        ]);
        assert!(matches!(value, HostValue::Array(items) if items.len() == 4));
    }

    #[test]
    fn host_value_json_fallback_preserves_bytes_and_resource_handles() {
        let values = HostValue::Array(vec![
            HostValue::Bytes(vec![0, 1, 127, 255]),
            HostValue::Resource(HostResourceHandle::new(9, 3, "image")),
        ]);
        assert_eq!(
            HostValue::from_json_str(&values.to_json_string()).unwrap(),
            values
        );
    }

    #[test]
    fn resource_registry_rejects_stale_generation_after_release() {
        let resources = HostResourceRegistry::new();
        let first = resources.insert("bytes", vec![1_u8, 2, 3]);
        assert_eq!(&*resources.get::<Vec<u8>>(&first).unwrap(), &[1, 2, 3]);
        assert!(resources.release(&first));
        assert!(resources.get::<Vec<u8>>(&first).is_none());

        let second = resources.insert("bytes", vec![4_u8]);
        assert_eq!(second.id, first.id);
        assert!(second.generation > first.generation);
        assert!(!resources.release(&first));
        assert_eq!(&*resources.get::<Vec<u8>>(&second).unwrap(), &[4]);
    }

    #[test]
    fn async_host_request_completes_once_and_honors_cancellation() {
        let resources = HostResourceRegistry::new();
        let context = HostRequestContext::new(
            HostRequestId(1),
            HostCancellationToken::default(),
            resources,
        );
        let (completion, pending) = context.pending();
        assert!(completion.resolve(HostValue::string("done")));
        assert_eq!(pending.try_take().unwrap().unwrap().as_str(), Some("done"));

        let cancelled = HostRequestContext::new(
            HostRequestId(2),
            HostCancellationToken::default(),
            HostResourceRegistry::new(),
        );
        let (completion, pending) = cancelled.pending();
        pending.cancel();
        assert!(!completion.resolve(HostValue::Null));
        assert!(pending.is_cancelled());
    }

    #[test]
    fn host_events_preserve_order_until_engine_drain() {
        let events = HostEventSender::default();
        events.send("frame", HostValue::Number(1.0));
        events.send("frame", HostValue::Number(2.0));
        let drained = events.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].payload.as_f64(), Some(1.0));
        assert_eq!(drained[1].payload.as_f64(), Some(2.0));
        assert!(events.drain().is_empty());
    }

    #[test]
    fn reliable_host_event_evicts_lossy_state_but_never_reliable_lifecycle() {
        let events = HostEventSender::with_capacity(2);
        events.send("frame", HostValue::Number(1.0));
        events
            .send_reliable("window-ready", HostValue::Number(7.0))
            .unwrap();
        events
            .send_reliable("window-closed", HostValue::Number(7.0))
            .unwrap();
        assert!(
            events
                .send_reliable("window-open-failed", HostValue::Number(8.0))
                .is_err()
        );
        let names = events
            .drain()
            .into_iter()
            .map(|event| event.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["window-ready", "window-closed"]);
    }

    #[test]
    fn host_api_registry_dispatches() {
        let mut api = HostApiRegistry::new();
        api.register("add", |args| {
            let a = args.first().and_then(HostValue::as_f64).unwrap_or(0.0);
            let b = args.get(1).and_then(HostValue::as_f64).unwrap_or(0.0);
            Ok(HostValue::Number(a + b))
        });
        let result = api
            .call("add", &[HostValue::Number(1.5), HostValue::Number(2.25)])
            .unwrap();
        assert_eq!(result.as_f64(), Some(3.75));
        assert!(api.call("missing", &[]).is_err());
    }

    #[test]
    fn host_api_registry_extend_is_atomic_on_conflict() {
        let mut framework = HostApiRegistry::new();
        framework.register("render", |_| Ok(HostValue::string("framework")));
        let mut application = HostApiRegistry::new();
        application
            .register("fetchRepos", |_| Ok(HostValue::Bool(true)))
            .register("render", |_| Ok(HostValue::string("application")));

        let error = framework.try_extend(&application).unwrap_err();
        assert_eq!(error.message, "duplicate host API name `render`");
        assert!(framework.get("fetchRepos").is_none());
        assert_eq!(
            framework.call("render", &[]).unwrap().as_str(),
            Some("framework")
        );
    }

    #[test]
    fn host_api_registry_extends_with_application_handlers() {
        let mut framework = HostApiRegistry::new();
        framework.register("render", |_| Ok(HostValue::Null));
        let mut application = HostApiRegistry::new();
        application.register("fetchRepos", |_| Ok(HostValue::Bool(true)));

        framework.try_extend(&application).unwrap();
        assert_eq!(
            framework.call("fetchRepos", &[]).unwrap().as_bool(),
            Some(true)
        );
    }

    #[test]
    fn host_api_registry_routes_async_handlers_through_invoke() {
        let mut api = HostApiRegistry::new();
        api.register_async("load", |args, context| {
            let value = args.first().cloned().unwrap_or(HostValue::Null);
            let (completion, pending) = context.pending();
            assert!(completion.resolve(value));
            Ok(pending)
        });
        assert!(api.call("load", &[]).is_err());
        let invocation = api.invoke(
            "load",
            vec![HostValue::Bytes(vec![4, 2])],
            HostRequestContext::new(
                HostRequestId(7),
                HostCancellationToken::default(),
                HostResourceRegistry::new(),
            ),
        );
        let HostInvocation::Pending(pending) = invocation else {
            panic!("async handler must return pending invocation");
        };
        assert_eq!(
            pending.try_take().unwrap().unwrap().as_bytes(),
            Some([4_u8, 2].as_slice())
        );
    }

    #[test]
    fn probe_artifact_is_nonempty_utf8() {
        let artifact = vue_runtime_probe_artifact();
        let source = artifact.source_utf8().unwrap();
        assert!(source.contains("__nanaProbe"));
        assert!(source.len() > 10_000);
    }

    #[test]
    fn probe_host_registry_records_ops() {
        let (api, state) = probe_host_registry();
        api.call("createElement", &[HostValue::string("div")])
            .unwrap();
        api.call("increment", &[HostValue::Number(2.0)]).unwrap();
        let snap = api.call("snapshot", &[]).unwrap();
        let map = snap.as_object().unwrap();
        assert_eq!(
            map.get("createElement").and_then(HostValue::as_f64),
            Some(1.0)
        );
        assert_eq!(map.get("lastCount").and_then(HostValue::as_f64), Some(2.0));
        assert_eq!(state.lock().unwrap().increment, 1);
    }

    #[test]
    fn module_specifier_and_ids_are_hashable() {
        use std::collections::HashSet;
        assert!(HashSet::from([JsFunctionId(1), JsFunctionId(2)]).contains(&JsFunctionId(1)));
        assert!(HashSet::from([JsObjectId(2)]).contains(&JsObjectId(2)));
        assert!(HashSet::from([JsWeakRef(3)]).contains(&JsWeakRef(3)));
        assert!(
            HashSet::from([ModuleSpecifier::new("nana:probe")])
                .contains(&ModuleSpecifier::new("nana:probe"))
        );
    }
}
