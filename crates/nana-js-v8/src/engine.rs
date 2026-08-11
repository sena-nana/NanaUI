//! Real V8 `JsEngine` implementation (feature = "engine").
//!
//! Bound to crates.io `v8 = "150.4.0"` (rusty_v8 successor package name).

use std::collections::BTreeMap;
use std::sync::{Condvar, Mutex, Once};

use nana_js_engine::{
    HostApiRegistry, HostValue, JsEngine, JsEngineError, JsException, JsFunctionId, RuntimeArtifact,
};

struct HostApiSlot {
    api: Mutex<HostApiRegistry>,
}

/// Serialize isolate create/drop vs `SnapshotCreator`.
///
/// V8's SnapshotCreator must not run while other isolates exist in-process; lib tests
/// that compile a snapshot in parallel with live `V8Engine`s otherwise SIGSEGV.
struct V8IsolateGate {
    live: Mutex<usize>,
    cv: Condvar,
}

fn isolate_gate() -> &'static V8IsolateGate {
    static GATE: V8IsolateGate = V8IsolateGate {
        live: Mutex::new(0),
        cv: Condvar::new(),
    };
    &GATE
}

/// V8 engine implementing [`JsEngine`].
pub struct V8Engine {
    isolate: Option<v8::OwnedIsolate>,
    context: Option<v8::Global<v8::Context>>,
    host_api: HostApiRegistry,
    functions: BTreeMap<u64, String>,
    next_function_id: u64,
    shut_down: bool,
}

impl Default for V8Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl V8Engine {
    pub fn new() -> Self {
        Self {
            isolate: None,
            context: None,
            host_api: HostApiRegistry::new(),
            functions: BTreeMap::new(),
            next_function_id: 1,
            shut_down: false,
        }
    }

    fn ensure_platform() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });
    }

    fn ensure_isolate(&mut self) -> Result<(), JsEngineError> {
        self.ensure_isolate_from_snapshot(None)
    }

    fn ensure_isolate_from_snapshot(
        &mut self,
        snapshot: Option<&[u8]>,
    ) -> Result<(), JsEngineError> {
        if self.shut_down {
            return Err(JsEngineError::new("V8Engine has been shut down"));
        }
        Self::ensure_platform();
        if self.isolate.is_none() {
            let gate = isolate_gate();
            let mut live = gate
                .live
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let params = match snapshot {
                Some(bytes) => {
                    let blob = v8::StartupData::from(bytes.to_vec());
                    v8::Isolate::create_params().snapshot_blob(blob)
                }
                None => v8::CreateParams::default(),
            };
            let mut isolate = v8::Isolate::new(params);
            isolate.set_slot(HostApiSlot {
                api: Mutex::new(self.host_api.clone()),
            });

            let global = {
                v8::scope!(let scope, &mut isolate);
                let context = v8::Context::new(scope, Default::default());
                let scope = &v8::ContextScope::new(scope, context);
                v8::Global::new(scope, context)
            };

            self.context = Some(global);
            self.isolate = Some(isolate);
            *live += 1;
            drop(live);
            self.install_shims_and_host()?;
        }
        Ok(())
    }

    fn drop_isolate(&mut self) {
        if self.isolate.is_none() && self.context.is_none() {
            return;
        }
        let gate = isolate_gate();
        let mut live = gate
            .live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.context = None;
        self.isolate = None;
        if *live > 0 {
            *live -= 1;
        }
        gate.cv.notify_all();
    }

    /// Compile UTF-8 JS into a V8 `StartupData` snapshot blob for Release embeds.
    ///
    /// Snapshot contents must be **host-free** (no `__nanaHost` / external callbacks).
    /// After [`JsEngine::initialize`] with [`RuntimeArtifactKind::V8Snapshot`], the
    /// engine installs the host bridge on the restored isolate. Full Vue/Lilia IIFEs
    /// that call host APIs at top-level are not snapshot-safe — use SourceUtf8 for
    /// those, or snapshot only pure prelude probes.
    ///
    /// Waits until no other in-process isolates are live (SnapshotCreator requirement).
    pub fn compile_snapshot(
        name: impl Into<String>,
        source: impl AsRef<[u8]>,
    ) -> Result<RuntimeArtifact, JsEngineError> {
        Self::ensure_platform();
        let name = name.into();
        let source = std::str::from_utf8(source.as_ref())
            .map_err(|err| JsEngineError::new(format!("V8 snapshot source is not UTF-8: {err}")))?
            .to_string();

        let gate = isolate_gate();
        let mut live = gate
            .live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while *live > 0 {
            live = gate
                .cv
                .wait(live)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }

        let mut snapshot_creator = v8::Isolate::snapshot_creator(None, None);
        {
            v8::scope!(let scope, &mut snapshot_creator);
            let context = v8::Context::new(scope, Default::default());
            {
                let scope = &mut v8::ContextScope::new(scope, context);
                eval_script(
                    scope,
                    r#"
                    if (typeof globalThis.process === "undefined") {
                      globalThis.process = { env: { NODE_ENV: "production" } };
                    }
                    if (typeof globalThis.console === "undefined") {
                      globalThis.console = { log() {}, warn() {}, error() {}, info() {}, debug() {} };
                    }
                    if (typeof globalThis.queueMicrotask !== "function") {
                      globalThis.queueMicrotask = function (fn) { Promise.resolve().then(fn); };
                    }
                    "#,
                )?;
                eval_script(scope, &source)?;
            }
            scope.set_default_context(context);
        }

        let blob = snapshot_creator
            .create_blob(v8::FunctionCodeHandling::Keep)
            .ok_or_else(|| JsEngineError::new("V8 SnapshotCreator::create_blob failed"))?;
        if blob.is_empty() {
            return Err(JsEngineError::new("V8 snapshot blob is empty"));
        }
        // Copy out before releasing the gate — StartupData may alias V8-owned memory.
        let bytes = blob.as_ref().to_vec();
        drop(blob);
        drop(live);
        Ok(RuntimeArtifact::from_v8_snapshot(name, bytes))
    }

    fn sync_host_slot(&mut self) -> Result<(), JsEngineError> {
        let isolate = self
            .isolate
            .as_mut()
            .ok_or_else(|| JsEngineError::new("V8 isolate missing"))?;
        if let Some(slot) = isolate.get_slot_mut::<HostApiSlot>() {
            *slot
                .api
                .lock()
                .map_err(|_| JsEngineError::new("host api slot poisoned"))? = self.host_api.clone();
        } else {
            isolate.set_slot(HostApiSlot {
                api: Mutex::new(self.host_api.clone()),
            });
        }
        Ok(())
    }

    fn install_shims_and_host(&mut self) -> Result<(), JsEngineError> {
        self.sync_host_slot()?;
        with_context(self, |scope| {
            install_host_bridge(scope)?;
            eval_script(
                scope,
                r#"
                if (typeof globalThis.process === "undefined") {
                  globalThis.process = { env: { NODE_ENV: "production" } };
                }
                if (typeof globalThis.console === "undefined") {
                  globalThis.console = { log() {}, warn() {}, error() {}, info() {}, debug() {} };
                }
                if (typeof globalThis.queueMicrotask !== "function") {
                  globalThis.queueMicrotask = function (fn) { Promise.resolve().then(fn); };
                }
                "#,
            )?;
            Ok(())
        })
    }
}

impl JsEngine for V8Engine {
    fn initialize(&mut self, artifact: RuntimeArtifact) -> Result<(), JsEngineError> {
        use nana_js_engine::RuntimeArtifactKind;
        match artifact.kind {
            RuntimeArtifactKind::QuickJsBytecode => {
                return Err(JsEngineError::new(
                    "V8Engine cannot load QuickJsBytecode artifacts (engine-native only)",
                ));
            }
            RuntimeArtifactKind::V8Snapshot => {
                if self.isolate.is_some() {
                    return Err(JsEngineError::new(
                        "V8Engine already initialized; V8Snapshot requires a fresh isolate",
                    ));
                }
                // Restore heap from StartupData, then install host bridge (host-free snapshot).
                self.ensure_isolate_from_snapshot(Some(&artifact.bytes))?;
                return Ok(());
            }
            RuntimeArtifactKind::SourceUtf8 => {}
        }
        self.ensure_isolate()?;
        let source = artifact.source_utf8()?.to_string();
        with_context(self, |scope| {
            install_host_bridge(scope)?;
            eval_script(
                scope,
                r#"
                if (typeof globalThis.process === "undefined") {
                  globalThis.process = { env: { NODE_ENV: "production" } };
                }
                if (typeof globalThis.console === "undefined") {
                  globalThis.console = { log() {}, warn() {}, error() {}, info() {}, debug() {} };
                }
                if (typeof globalThis.queueMicrotask !== "function") {
                  globalThis.queueMicrotask = function (fn) { Promise.resolve().then(fn); };
                }
                "#,
            )?;
            eval_script(scope, &source)?;
            Ok(())
        })
    }

    fn register_host_api(&mut self, api: &HostApiRegistry) -> Result<(), JsEngineError> {
        self.host_api = api.clone();
        if self.isolate.is_some() {
            self.sync_host_slot()?;
            with_context(self, |scope| install_host_bridge(scope))?;
        }
        Ok(())
    }

    fn resolve_function(&mut self, name: &str) -> Result<JsFunctionId, JsEngineError> {
        self.ensure_isolate()?;
        let name_owned = name.to_string();
        with_context(self, |scope| {
            let value = lookup_path(scope, &name_owned)?;
            if !value.is_function() {
                return Err(JsEngineError::new(format!(
                    "`{name_owned}` is not a function"
                )));
            }
            Ok(())
        })?;
        let id = self.next_function_id;
        self.next_function_id += 1;
        self.functions.insert(id, name_owned);
        Ok(JsFunctionId(id))
    }

    fn invoke(
        &mut self,
        target: JsFunctionId,
        args: &[HostValue],
    ) -> Result<HostValue, JsEngineError> {
        let name = self
            .functions
            .get(&target.0)
            .cloned()
            .ok_or_else(|| JsEngineError::new(format!("unknown JsFunctionId {}", target.0)))?;
        with_context(self, |scope| {
            let value = lookup_path(scope, &name)?;
            let func = v8::Local::<v8::Function>::try_from(value)
                .map_err(|_| JsEngineError::new(format!("`{name}` is not a function")))?;
            let recv = v8::undefined(scope).into();
            let mut js_args = Vec::with_capacity(args.len());
            for arg in args {
                js_args.push(host_to_v8(scope, arg)?);
            }
            v8::tc_scope!(let try_catch, scope);
            match func.call(try_catch, recv, &js_args) {
                Some(result) => v8_to_host(try_catch, result),
                None => Err(exception_from_try_catch(try_catch)),
            }
        })
    }

    fn run_microtasks(&mut self) -> Result<(), JsEngineError> {
        if let Some(isolate) = self.isolate.as_mut() {
            isolate.perform_microtask_checkpoint();
        }
        Ok(())
    }

    fn interrupt(&mut self) {
        if let Some(isolate) = self.isolate.as_mut() {
            isolate.terminate_execution();
        }
    }

    fn request_gc(&mut self) {
        if let Some(isolate) = self.isolate.as_mut() {
            isolate.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
        }
    }

    fn shutdown(&mut self) {
        self.functions.clear();
        self.drop_isolate();
        self.shut_down = true;
    }
}

impl Drop for V8Engine {
    fn drop(&mut self) {
        self.drop_isolate();
    }
}

fn with_context<T>(
    engine: &mut V8Engine,
    f: impl for<'s> FnOnce(&mut v8::PinScope<'s, '_>) -> Result<T, JsEngineError>,
) -> Result<T, JsEngineError> {
    engine.ensure_isolate()?;
    let context = engine
        .context
        .as_ref()
        .ok_or_else(|| JsEngineError::new("V8 context missing"))?
        .clone();
    let isolate = engine
        .isolate
        .as_mut()
        .ok_or_else(|| JsEngineError::new("V8 isolate missing"))?;

    v8::scope!(let handle_scope, isolate);
    let local_context = v8::Local::new(handle_scope, context);
    let scope = &mut v8::ContextScope::new(handle_scope, local_context);
    f(scope)
}

fn install_host_bridge(scope: &mut v8::PinScope) -> Result<(), JsEngineError> {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "__nanaHostRaw")
        .ok_or_else(|| JsEngineError::new("failed to allocate __nanaHostRaw string"))?;
    let tmpl = v8::FunctionTemplate::new(scope, host_raw_callback);
    let func = tmpl
        .get_function(scope)
        .ok_or_else(|| JsEngineError::new("failed to create __nanaHostRaw function"))?;
    global.set(scope, key.into(), func.into());

    eval_script(
        scope,
        r#"
        globalThis.__nanaHost = {
          call(name, args) {
            const raw = globalThis.__nanaHostRaw(String(name), JSON.stringify(args ?? []));
            const parsed = JSON.parse(raw);
            if (parsed && typeof parsed === "object" && Object.prototype.hasOwnProperty.call(parsed, "__nanaHostError")) {
              throw new Error(String(parsed.__nanaHostError));
            }
            return parsed;
          }
        };
        "#,
    )?;
    Ok(())
}

fn host_raw_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let args_json = args
        .get(1)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "[]".into());

    let result = match scope.get_slot::<HostApiSlot>() {
        Some(slot) => match slot.api.lock() {
            Ok(api) => {
                let args_value =
                    HostValue::from_json_str(&args_json).unwrap_or(HostValue::Array(vec![]));
                let host_args = match args_value {
                    HostValue::Array(items) => items,
                    other => vec![other],
                };
                match api.call(&name, &host_args) {
                    Ok(value) => value.to_json_string(),
                    Err(exception) => format!(
                        "{{\"__nanaHostError\":{}}}",
                        serde_json::Value::String(exception.message)
                    ),
                }
            }
            Err(_) => "{\"__nanaHostError\":\"host api slot poisoned\"}".into(),
        },
        None => "{\"__nanaHostError\":\"host api slot missing\"}".into(),
    };

    if let Some(s) = v8::String::new(scope, &result) {
        rv.set(s.into());
    }
}

fn eval_script(scope: &mut v8::PinScope, source: &str) -> Result<(), JsEngineError> {
    let code = v8::String::new(scope, source)
        .ok_or_else(|| JsEngineError::new("failed to allocate script string"))?;
    v8::tc_scope!(let try_catch, scope);
    let script = v8::Script::compile(try_catch, code, None)
        .ok_or_else(|| exception_from_try_catch(try_catch))?;
    script
        .run(try_catch)
        .ok_or_else(|| exception_from_try_catch(try_catch))?;
    Ok(())
}

fn lookup_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, JsEngineError> {
    let mut current: v8::Local<'s, v8::Value> = scope.get_current_context().global(scope).into();
    for (index, segment) in name.split('.').enumerate() {
        let key = v8::String::new(scope, segment)
            .ok_or_else(|| JsEngineError::new("failed to allocate property name"))?;
        let obj = current
            .to_object(scope)
            .ok_or_else(|| JsEngineError::new(format!("`{name}` is not an object path")))?;
        let next = obj.get(scope, key.into()).ok_or_else(|| {
            JsEngineError::new(format!(
                "export `{name}` missing at segment `{segment}` (index {index})"
            ))
        })?;
        if next.is_null_or_undefined() {
            return Err(JsEngineError::new(format!(
                "export `{name}` missing at segment `{segment}` (index {index})"
            )));
        }
        current = next;
    }
    Ok(current)
}

fn host_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &HostValue,
) -> Result<v8::Local<'s, v8::Value>, JsEngineError> {
    let json = value.to_json_string();
    let code = format!("({json})");
    let source = v8::String::new(scope, &code)
        .ok_or_else(|| JsEngineError::new("failed to allocate JSON literal"))?;
    v8::tc_scope!(let try_catch, scope);
    let script = v8::Script::compile(try_catch, source, None)
        .ok_or_else(|| exception_from_try_catch(try_catch))?;
    script
        .run(try_catch)
        .ok_or_else(|| exception_from_try_catch(try_catch))
}

fn v8_to_host(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<HostValue, JsEngineError> {
    if value.is_null() {
        return Ok(HostValue::Null);
    }
    if value.is_undefined() {
        return Ok(HostValue::Undefined);
    }
    if value.is_boolean() {
        return Ok(HostValue::Bool(value.boolean_value(scope)));
    }
    if value.is_number() {
        return Ok(HostValue::Number(
            value.number_value(scope).unwrap_or(f64::NAN),
        ));
    }
    if value.is_string() {
        let s = value
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default();
        return Ok(HostValue::String(s));
    }

    let json = v8::json::stringify(scope, value)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "null".into());
    HostValue::from_json_str(&json)
}

fn exception_from_try_catch(
    try_catch: &mut v8::PinnedRef<'_, v8::TryCatch<'_, '_, v8::HandleScope>>,
) -> JsEngineError {
    if let Some(exception) = try_catch.exception() {
        let message = exception
            .to_string(try_catch)
            .map(|s| s.to_rust_string_lossy(try_catch))
            .unwrap_or_else(|| "unknown V8 exception".into());
        let stack = try_catch
            .stack_trace()
            .and_then(|s| s.to_string(try_catch))
            .map(|s| s.to_rust_string_lossy(try_catch));
        return JsEngineError::from_exception(JsException { message, stack });
    }
    if let Some(message) = try_catch.message() {
        return JsEngineError::new(message.get(try_catch).to_rust_string_lossy(try_catch));
    }
    JsEngineError::new("unknown V8 error")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_js_engine::probe::{probe_host_registry, vue_runtime_probe_artifact};
    use std::sync::Mutex;

    /// Lib tests share one process; serialize so SnapshotCreator never races peers.
    fn with_serial_v8_tests<R>(f: impl FnOnce() -> R) -> R {
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        f()
    }

    #[test]
    fn compile_and_load_v8_snapshot_without_plaintext() {
        with_serial_v8_tests(|| {
            let source = r#"
            globalThis.__nanaSnapshotProbe = {
              run: () => ({ ok: true, via: "v8-snapshot", n: 1 + 1 })
            };
        "#;
            let artifact =
                V8Engine::compile_snapshot("probe.v8snap.js", source).expect("compile V8 snapshot");
            assert_eq!(
                artifact.kind,
                nana_js_engine::RuntimeArtifactKind::V8Snapshot
            );
            assert!(artifact.is_binary_release());
            assert!(artifact.source_utf8().is_err());
            assert!(!artifact.bytes.is_empty());

            let mut engine = V8Engine::new();
            engine.initialize(artifact).expect("load V8 snapshot");
            let run = engine
                .resolve_function("__nanaSnapshotProbe.run")
                .expect("export");
            let result = engine.invoke(run, &[]).expect("invoke");
            let obj = result.as_object().expect("object");
            assert_eq!(obj.get("ok").and_then(HostValue::as_bool), Some(true));
            assert_eq!(obj.get("n").and_then(HostValue::as_f64), Some(2.0));
            assert_eq!(
                obj.get("via").and_then(HostValue::as_str),
                Some("v8-snapshot")
            );
            engine.shutdown();
        });
    }

    #[test]
    fn evaluates_simple_script_and_host_callback() {
        with_serial_v8_tests(|| {
            let mut engine = V8Engine::new();
            let mut api = HostApiRegistry::new();
            api.register("echo", |args| {
                Ok(args.first().cloned().unwrap_or(HostValue::Null))
            });
            engine.register_host_api(&api).unwrap();
            engine
                .initialize(RuntimeArtifact::from_source(
                    "echo.js",
                    "globalThis.__nanaProbe = { run: () => globalThis.__nanaHost.call('echo', ['ok']) };",
                ))
                .unwrap();
            let run = engine.resolve_function("__nanaProbe.run").unwrap();
            let result = engine.invoke(run, &[]).unwrap();
            assert_eq!(result.as_str(), Some("ok"));
            engine.shutdown();
        });
    }

    #[test]
    fn runs_shared_vue_runtime_probe() {
        with_serial_v8_tests(|| {
            let mut engine = V8Engine::new();
            let (api, state) = probe_host_registry();
            engine.register_host_api(&api).unwrap();
            engine
                .initialize(vue_runtime_probe_artifact())
                .expect("vue probe should initialize on V8");
            let run = engine
                .resolve_function("__nanaProbe.run")
                .expect("probe export");
            let result = engine.invoke(run, &[]).expect("probe run");
            engine.run_microtasks().unwrap();

            let object = result.as_object().expect("probe returns object");
            assert_eq!(object.get("ok").and_then(HostValue::as_bool), Some(true));
            assert_eq!(object.get("vue").and_then(HostValue::as_bool), Some(true));
            assert_eq!(object.get("count").and_then(HostValue::as_f64), Some(2.0));

            let guard = state.lock().unwrap();
            assert!(guard.create_element >= 1, "{guard:?}");
            assert!(guard.insert >= 1, "{guard:?}");
            assert!(guard.increment >= 1, "{guard:?}");
            assert_eq!(guard.last_count, 2);
            drop(guard);
            engine.shutdown();
        });
    }
}
