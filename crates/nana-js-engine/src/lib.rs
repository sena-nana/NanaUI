//! Engine-agnostic JavaScript host interface for NanaUI Vue.
//!
//! Concrete QuickJS / V8 types must not leak through this crate. Applications and
//! `nana-ui-vue` depend only on these types; each app links exactly one of
//! `nana-js-quickjs` or `nana-js-v8`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Opaque handle for a JS function retained by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JsFunctionId(pub u64);

/// Opaque handle for a JS object retained by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JsObjectId(pub u64);

/// Weak reference placeholder for Phase 3 DOM/node handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JsWeakRef(pub u64);

/// Module specifier string used when loading ES modules (Phase 3+).
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
    String(String),
    Array(Vec<HostValue>),
    Object(BTreeMap<String, HostValue>),
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

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, HostValue>> {
        match self {
            Self::Object(map) => Some(map),
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
            Self::String(v) => serde_json::Value::String(v.clone()),
            Self::Array(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json_value).collect())
            }
            Self::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), v.to_json_value()))
                    .collect(),
            ),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsException {
    pub message: String,
    pub stack: Option<String>,
}

impl JsException {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            stack: None,
        }
    }

    pub fn with_stack(message: impl Into<String>, stack: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            stack: Some(stack.into()),
        }
    }
}

impl fmt::Display for JsException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsException {}

/// Engine-level failure (init, invoke, conversion, or JS exception).
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Host API callback invoked from JavaScript via `__nanaHost.call(name, args)`.
pub type HostApiHandler = Arc<dyn Fn(&[HostValue]) -> Result<HostValue, JsException> + Send + Sync>;

/// Registry of named host callbacks shared by QuickJS and V8 backends.
#[derive(Default, Clone)]
pub struct HostApiRegistry {
    handlers: BTreeMap<String, HostApiHandler>,
}

impl HostApiRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        handler: impl Fn(&[HostValue]) -> Result<HostValue, JsException> + Send + Sync + 'static,
    ) -> &mut Self {
        self.handlers.insert(name.into(), Arc::new(handler));
        self
    }

    pub fn register_arc(&mut self, name: impl Into<String>, handler: HostApiHandler) -> &mut Self {
        self.handlers.insert(name.into(), handler);
        self
    }

    pub fn get(&self, name: &str) -> Option<&HostApiHandler> {
        self.handlers.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.handlers.keys().map(String::as_str)
    }

    pub fn call(&self, name: &str, args: &[HostValue]) -> Result<HostValue, JsException> {
        let handler = self
            .handlers
            .get(name)
            .ok_or_else(|| JsException::new(format!("unknown host API `{name}`")))?;
        handler(args)
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }
}

impl fmt::Debug for HostApiRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostApiRegistry")
            .field("handlers", &self.handlers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Unified JS engine contract used by `nana-ui-vue`.
///
/// Phase 3 will keep this surface and add DOM/Custom Renderer wiring on top;
/// Blitz paint stays outside this trait.
pub trait JsEngine {
    /// Evaluate / load a runtime artifact (Phase 2: UTF-8 JS source).
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

    fn interrupt(&mut self);
    fn request_gc(&mut self);
    fn shutdown(&mut self);
}

/// Shared Phase 2 Vue `runtime-core` probe artifact (IIFE, UTF-8).
pub mod probe {
    use super::{HostApiRegistry, HostValue, JsException, RuntimeArtifact};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// Pre-bundled `@vue/runtime-core` probe used by both engines (Phase 2 stub host ops).
    pub const VUE_RUNTIME_PROBE_JS: &str =
        include_str!("../fixtures/vue-runtime-probe/dist/vue-runtime-probe.iife.js");

    /// Phase 3 Custom Renderer apps (Counter/Todo) — hostOps return Rust DOM handles.
    pub const VUE_PHASE3_JS: &str =
        include_str!("../fixtures/vue-runtime-probe/dist/vue-phase3.iife.js");

    pub fn vue_runtime_probe_artifact() -> RuntimeArtifact {
        RuntimeArtifact::from_source("vue-runtime-probe.iife.js", VUE_RUNTIME_PROBE_JS)
    }

    pub fn vue_phase3_artifact() -> RuntimeArtifact {
        RuntimeArtifact::from_source("vue-phase3.iife.js", VUE_PHASE3_JS)
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
