//! QuickJS-NG backend via `rquickjs = "0.12.2"`.
//!
//! Do not pull V8 into this crate. Applications choose QuickJS XOR V8.

use std::collections::BTreeMap;

use nana_js_engine::{
    HostApiRegistry, HostValue, JsEngine, JsEngineError, JsException, JsFunctionId,
    RuntimeArtifact, RuntimeArtifactKind,
};
use rquickjs::{
    Array, BigInt, Context, Ctx, Function, Module, Object, Runtime, Value, WriteOptions,
    function::Args as JsArgs,
};

/// QuickJS engine implementing [`JsEngine`].
pub struct QuickJsEngine {
    runtime: Option<Runtime>,
    context: Option<Context>,
    host_api: HostApiRegistry,
    /// Function ids map to global property paths (for example `__nanaProbe.run`).
    functions: BTreeMap<u64, String>,
    next_function_id: u64,
    shut_down: bool,
}

impl Default for QuickJsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl QuickJsEngine {
    pub fn new() -> Self {
        Self {
            runtime: None,
            context: None,
            host_api: HostApiRegistry::new(),
            functions: BTreeMap::new(),
            next_function_id: 1,
            shut_down: false,
        }
    }

    /// Compile UTF-8 JS (script or IIFE) to QuickJS module bytecode for Release embeds.
    ///
    /// The source is declared as an ES module (side effects + `globalThis` exports).
    /// Resulting bytes are **not** interchangeable with V8 snapshots.
    pub fn compile_bytecode(
        name: impl Into<String>,
        source: impl AsRef<[u8]>,
    ) -> Result<RuntimeArtifact, JsEngineError> {
        let name = name.into();
        let source = source.as_ref().to_vec();
        let runtime = Runtime::new().map_err(|err| {
            JsEngineError::new(format!("failed to create QuickJS runtime: {err}"))
        })?;
        let context = Context::full(&runtime).map_err(|err| {
            JsEngineError::new(format!("failed to create QuickJS context: {err}"))
        })?;
        let bytes = context.with(|ctx| -> Result<Vec<u8>, JsEngineError> {
            // Ensure module body can reference process/console during compile of Vue IIFEs.
            ctx.eval::<(), _>(
                br#"
                if (typeof globalThis.process === "undefined") {
                  globalThis.process = { env: { NODE_ENV: "production" } };
                }
                if (typeof globalThis.console === "undefined") {
                  globalThis.console = { log() {}, warn() {}, error() {}, info() {}, debug() {} };
                }
                "#,
            )
            .map_err(|err| map_eval_error(&ctx, err))?;
            let module = Module::declare(ctx.clone(), name.as_str(), source)
                .map_err(|err| map_eval_error(&ctx, err))?;
            module
                .write(WriteOptions::default())
                .map_err(|err| map_eval_error(&ctx, err))
        })?;
        Ok(RuntimeArtifact::from_quickjs_bytecode(name, bytes))
    }

    fn eval_shims(ctx: &Ctx<'_>) -> Result<(), JsEngineError> {
        ctx.eval::<(), _>(
            br#"
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
        )
        .map_err(|err| map_eval_error(ctx, err))
    }

    fn load_bytecode(ctx: &Ctx<'_>, bytes: &[u8]) -> Result<(), JsEngineError> {
        // SAFETY: bytes come from QuickJsEngine::compile_bytecode / Module::write.
        let module =
            unsafe { Module::load(ctx.clone(), bytes) }.map_err(|err| map_eval_error(ctx, err))?;
        let (_evaluated, promise) = module.eval().map_err(|err| map_eval_error(ctx, err))?;
        promise
            .finish::<()>()
            .map_err(|err| map_eval_error(ctx, err))?;
        Ok(())
    }

    fn ensure_runtime(&mut self) -> Result<(), JsEngineError> {
        if self.shut_down {
            return Err(JsEngineError::new("QuickJsEngine has been shut down"));
        }
        if self.runtime.is_none() {
            let runtime = Runtime::new().map_err(|err| {
                JsEngineError::new(format!("failed to create QuickJS runtime: {err}"))
            })?;
            let context = Context::full(&runtime).map_err(|err| {
                JsEngineError::new(format!("failed to create QuickJS context: {err}"))
            })?;
            self.runtime = Some(runtime);
            self.context = Some(context);
        }
        Ok(())
    }

    fn with_ctx<T>(
        &mut self,
        f: impl FnOnce(Ctx<'_>) -> Result<T, JsEngineError>,
    ) -> Result<T, JsEngineError> {
        self.ensure_runtime()?;
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| JsEngineError::new("QuickJS context missing"))?;
        context.with(f)
    }

    fn install_host_bridge(ctx: &Ctx<'_>, api: &HostApiRegistry) -> Result<(), JsEngineError> {
        let api = api.clone();
        // Stringly bridge avoids rquickjs `Value<'js>` lifetime traps in host callbacks.
        let raw = Function::new(ctx.clone(), move |name: String, args_json: String| {
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
        })
        .map_err(map_rquickjs_error)?;

        ctx.globals()
            .set("__nanaHostRaw", raw)
            .map_err(map_rquickjs_error)?;
        ctx.eval::<(), _>(nana_js_engine::HOST_BRIDGE_INSTALL_JS.as_bytes())
            .map_err(|err| map_eval_error(ctx, err))?;
        Ok(())
    }

    fn lookup_path<'js>(ctx: &Ctx<'js>, name: &str) -> Result<Value<'js>, JsEngineError> {
        let path: Vec<&str> = name.split('.').collect();
        let mut current = ctx.globals().into_value();
        for (index, segment) in path.iter().enumerate() {
            let obj = current
                .as_object()
                .ok_or_else(|| JsEngineError::new(format!("`{name}` is not an object path")))?;
            let next: Value<'js> = obj.get(*segment).map_err(map_rquickjs_error)?;
            if next.is_undefined() || next.is_null() {
                return Err(JsEngineError::new(format!(
                    "export `{name}` missing at segment `{segment}` (index {index})"
                )));
            }
            current = next;
        }
        Ok(current)
    }
}

impl Drop for QuickJsEngine {
    fn drop(&mut self) {
        self.functions.clear();
        // Drop context before runtime so QuickJS gc lists stay consistent.
        self.context = None;
        self.runtime = None;
    }
}

impl JsEngine for QuickJsEngine {
    fn initialize(&mut self, artifact: RuntimeArtifact) -> Result<(), JsEngineError> {
        self.ensure_runtime()?;
        match artifact.kind {
            RuntimeArtifactKind::V8Snapshot => Err(JsEngineError::new(
                "QuickJsEngine cannot load V8Snapshot artifacts (engine-native only)",
            )),
            RuntimeArtifactKind::QuickJsBytecode => {
                let bytes = artifact.bytes.clone();
                let host_api = self.host_api.clone();
                self.with_ctx(|ctx| {
                    Self::eval_shims(&ctx)?;
                    if !host_api.is_empty() {
                        Self::install_host_bridge(&ctx, &host_api)?;
                    }
                    Self::load_bytecode(&ctx, &bytes)
                })
            }
            RuntimeArtifactKind::SourceUtf8 => {
                let source = artifact.source_utf8()?.to_string();
                let host_api = self.host_api.clone();
                self.with_ctx(|ctx| {
                    Self::eval_shims(&ctx)?;
                    if !host_api.is_empty() {
                        Self::install_host_bridge(&ctx, &host_api)?;
                    }
                    ctx.eval::<(), _>(source.as_bytes())
                        .map_err(|err| map_eval_error(&ctx, err))?;
                    Ok(())
                })
            }
        }
    }

    fn register_host_api(&mut self, api: &HostApiRegistry) -> Result<(), JsEngineError> {
        self.host_api = api.clone();
        if self.context.is_some() {
            let host_api = self.host_api.clone();
            self.with_ctx(|ctx| Self::install_host_bridge(&ctx, &host_api))?;
        }
        Ok(())
    }

    fn resolve_function(&mut self, name: &str) -> Result<JsFunctionId, JsEngineError> {
        // Validate the export exists now; store the path for later invoke.
        let name_owned = name.to_string();
        self.with_ctx(|ctx| {
            let value = Self::lookup_path(&ctx, &name_owned)?;
            if value.as_function().is_none() {
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

        self.with_ctx(|ctx| {
            let value = Self::lookup_path(&ctx, &name)?;
            let func = value
                .into_function()
                .ok_or_else(|| JsEngineError::new(format!("`{name}` is not a function")))?;
            let mut js_args = JsArgs::new(ctx.clone(), args.len());
            for arg in args {
                let value = host_to_js(&ctx, arg)?;
                js_args.push_arg(value).map_err(map_rquickjs_error)?;
            }
            let result: Value<'_> = func
                .call_arg(js_args)
                .map_err(|err| map_eval_error(&ctx, err))?;
            js_value_to_host(&ctx, result)
        })
    }

    fn run_microtasks(&mut self) -> Result<(), JsEngineError> {
        if let Some(runtime) = self.runtime.as_ref() {
            loop {
                match runtime.execute_pending_job() {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(err) => {
                        return Err(JsEngineError::new(format!(
                            "QuickJS pending job failed: {err}"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn interrupt(&mut self) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.set_interrupt_handler(Some(Box::new(|| true)));
        }
    }

    fn request_gc(&mut self) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.run_gc();
        }
    }

    fn shutdown(&mut self) {
        self.functions.clear();
        self.context = None;
        self.runtime = None;
        self.shut_down = true;
    }
}

fn map_rquickjs_error(err: rquickjs::Error) -> JsEngineError {
    JsEngineError::new(format!("rquickjs error: {err}"))
}

fn map_eval_error(ctx: &Ctx<'_>, err: rquickjs::Error) -> JsEngineError {
    if matches!(err, rquickjs::Error::Exception) {
        let caught = ctx.catch();
        let message = caught
            .as_object()
            .and_then(|obj| obj.get::<_, String>("message").ok())
            .unwrap_or_else(|| format!("{caught:?}"));
        let stack = caught
            .as_object()
            .and_then(|obj| obj.get::<_, String>("stack").ok());
        return JsEngineError::from_exception(JsException {
            name: "Error".into(),
            code: None,
            message,
            stack,
            details: None,
        });
    }
    map_rquickjs_error(err)
}

fn host_to_js<'js>(ctx: &Ctx<'js>, value: &HostValue) -> Result<Value<'js>, JsEngineError> {
    match value {
        HostValue::Null => Ok(Value::new_null(ctx.clone())),
        HostValue::Undefined => Ok(Value::new_undefined(ctx.clone())),
        HostValue::Bool(v) => Ok(Value::new_bool(ctx.clone(), *v)),
        HostValue::Number(v) => Ok(Value::new_number(ctx.clone(), *v)),
        HostValue::BigInt(v) => BigInt::from_u64(ctx.clone(), *v)
            .map(Into::into)
            .map_err(map_rquickjs_error),
        HostValue::String(v) => rquickjs::String::from_str(ctx.clone(), v)
            .map(Value::from)
            .map_err(map_rquickjs_error),
        HostValue::Bytes(bytes) => {
            let array = Array::new(ctx.clone()).map_err(map_rquickjs_error)?;
            for (index, byte) in bytes.iter().enumerate() {
                array.set(index, *byte).map_err(map_rquickjs_error)?;
            }
            Ok(array.into_value())
        }
        HostValue::Array(items) => {
            let array = Array::new(ctx.clone()).map_err(map_rquickjs_error)?;
            for (index, item) in items.iter().enumerate() {
                let js = host_to_js(ctx, item)?;
                array.set(index, js).map_err(map_rquickjs_error)?;
            }
            Ok(array.into_value())
        }
        HostValue::Object(map) => {
            let object = Object::new(ctx.clone()).map_err(map_rquickjs_error)?;
            for (key, item) in map {
                let js = host_to_js(ctx, item)?;
                object.set(key.as_str(), js).map_err(map_rquickjs_error)?;
            }
            Ok(object.into_value())
        }
        HostValue::Resource(handle) => {
            let object = Object::new(ctx.clone()).map_err(map_rquickjs_error)?;
            object.set("__resource", true).map_err(map_rquickjs_error)?;
            object
                .set("id", handle.id.to_string())
                .map_err(map_rquickjs_error)?;
            object
                .set("generation", handle.generation)
                .map_err(map_rquickjs_error)?;
            object
                .set("kind", handle.kind.as_str())
                .map_err(map_rquickjs_error)?;
            Ok(object.into_value())
        }
        HostValue::Function(id) => Err(JsEngineError::new(format!(
            "cannot pass JsFunctionId({}) into QuickJS yet",
            id.0
        ))),
        HostValue::ObjectRef(id) => Err(JsEngineError::new(format!(
            "cannot pass JsObjectId({}) into QuickJS yet",
            id.0
        ))),
    }
}

fn js_value_to_host<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<HostValue, JsEngineError> {
    if value.is_null() {
        return Ok(HostValue::Null);
    }
    if value.is_undefined() {
        return Ok(HostValue::Undefined);
    }
    if let Some(v) = value.as_bool() {
        return Ok(HostValue::Bool(v));
    }
    if let Some(v) = value.as_number() {
        return Ok(HostValue::Number(v));
    }
    if value.is_big_int() {
        // QuickJS-NG exposes a signed extraction helper. Preserve the common
        // non-negative resource-id range; V8 remains the production backend.
        let bigint = value
            .into_big_int()
            .ok_or_else(|| JsEngineError::new("invalid QuickJS BigInt"))?;
        let value = bigint.to_i64().map_err(map_rquickjs_error)?;
        return value
            .try_into()
            .map(HostValue::BigInt)
            .map_err(|_| JsEngineError::new("negative BigInt cannot cross the host bridge"));
    }
    // Arrays/objects before string: `get::<String>()` ToString-coerces arrays of
    // objects to "[object Object],[object Object]" (Vue reactive Proxies included).
    if let Some(array) = value.as_array() {
        let mut items = Vec::new();
        let len = array.len();
        for index in 0..len {
            let item: Value<'_> = array.get(index).map_err(map_rquickjs_error)?;
            items.push(js_value_to_host(ctx, item)?);
        }
        return Ok(HostValue::Array(items));
    }
    if value.is_object() {
        // Prefer JSON.stringify so Vue Proxies round-trip as plain data
        // (same approach as the V8 backend).
        if let Some(host) = json_stringify_to_host(ctx, &value) {
            return Ok(host);
        }
        if let Some(object) = value.as_object() {
            if let Some(len) = object_array_len(object) {
                let mut items = Vec::with_capacity(len);
                for index in 0..len {
                    let item: Value<'_> =
                        object.get(index.to_string()).map_err(map_rquickjs_error)?;
                    items.push(js_value_to_host(ctx, item)?);
                }
                return Ok(HostValue::Array(items));
            }
            let mut map = BTreeMap::new();
            for key in object.keys::<String>() {
                let key = key.map_err(map_rquickjs_error)?;
                if key.starts_with("__v_") {
                    continue;
                }
                let item: Value<'_> = object.get(key.as_str()).map_err(map_rquickjs_error)?;
                map.insert(key, js_value_to_host(ctx, item)?);
            }
            return Ok(HostValue::Object(map));
        }
    }
    if let Ok(v) = value.get::<String>()
        && v != "[object Object]"
        && !v.contains("[object Object]")
    {
        return Ok(HostValue::String(v));
    }
    Ok(HostValue::Null)
}

fn json_stringify_to_host<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Option<HostValue> {
    let global = ctx.globals();
    let json: Object<'js> = global.get("JSON").ok()?;
    let stringify: Function<'js> = json.get("stringify").ok()?;
    let mut args = JsArgs::new(ctx.clone(), 1);
    args.push_arg(value.clone()).ok()?;
    let json_val: Value<'js> = stringify.call_arg(args).ok()?;
    let s = json_val.get::<String>().ok()?;
    if s.is_empty() || s == "null" || s == "undefined" {
        return None;
    }
    HostValue::from_json_str(&s).ok()
}

fn object_array_len<'js>(object: &Object<'js>) -> Option<usize> {
    let len: Value<'js> = object.get("length").ok()?;
    let len = len.as_number()?;
    if !len.is_finite() || len < 0.0 {
        return None;
    }
    let len = len as usize;
    if len == 0 {
        return None;
    }
    let zero: Result<Value<'js>, _> = object.get("0");
    zero.ok().map(|_| len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_js_engine::probe::{
        VUE_SFC_COMPAT_CSS, probe_host_registry, vue_runtime_probe_artifact,
        vue_sfc_compat_artifact,
    };
    use nana_js_engine::{HostValue, JsEngine, RuntimeArtifactKind};

    #[test]
    fn evaluates_simple_script_and_host_callback() {
        let mut engine = QuickJsEngine::new();
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
    }

    #[test]
    fn navigator_clipboard_write_read_via_shim_and_memory_host() {
        use nana_ui_web_api::{
            MemoryClipboard, WEB_API_SHIM_JS, register_web_api_host_ops_with_clipboard,
            shared_clipboard, shared_web_api_state,
        };
        use std::sync::Arc;

        let clipboard = shared_clipboard(MemoryClipboard::new());
        let mut api = HostApiRegistry::new();
        register_web_api_host_ops_with_clipboard(
            &mut api,
            shared_web_api_state(),
            Arc::clone(&clipboard),
        );

        let mut engine = QuickJsEngine::new();
        engine.register_host_api(&api).unwrap();
        let source = format!(
            "{WEB_API_SHIM_JS}\n\
             globalThis.__clip = {{ done: false, value: null, err: null }};\n\
             globalThis.__nanaClipboardProbe = {{\n\
               start: function () {{\n\
                 navigator.clipboard.writeText('nana-qjs-clipboard')\n\
                   .then(function () {{ return navigator.clipboard.readText(); }})\n\
                   .then(function (text) {{\n\
                     globalThis.__clip.done = true;\n\
                     globalThis.__clip.value = text;\n\
                   }})\n\
                   .catch(function (e) {{\n\
                     globalThis.__clip.done = true;\n\
                     globalThis.__clip.err = String((e && e.message) || e);\n\
                   }});\n\
               }},\n\
               status: function () {{ return globalThis.__clip; }}\n\
             }};\n"
        );
        engine
            .initialize(RuntimeArtifact::from_source("clipboard-probe.js", source))
            .unwrap();
        let start = engine
            .resolve_function("__nanaClipboardProbe.start")
            .unwrap();
        let status = engine
            .resolve_function("__nanaClipboardProbe.status")
            .unwrap();
        engine.invoke(start, &[]).unwrap();
        for _ in 0..32 {
            engine.run_microtasks().unwrap();
            let snap = engine.invoke(status, &[]).unwrap();
            let obj = snap.as_object().expect("status object");
            if obj.get("done").and_then(HostValue::as_bool) == Some(true) {
                assert_eq!(
                    obj.get("err").and_then(|v| match v {
                        HostValue::Null | HostValue::Undefined => None,
                        other => Some(other.to_json_string()),
                    }),
                    None,
                    "clipboard promise rejected: {obj:?}"
                );
                assert_eq!(
                    obj.get("value").and_then(HostValue::as_str),
                    Some("nana-qjs-clipboard")
                );
                engine.shutdown();
                return;
            }
        }
        panic!("navigator.clipboard promise did not settle");
    }

    #[test]
    fn buffered_fetch_classes_enforce_body_and_abort_contracts() {
        use nana_ui_vue::VueHost;

        let mut host = VueHost::with_viewport(320, 200, 1.0);
        let mut engine = QuickJsEngine::new();
        host.initialize_with_web_api(
            &mut engine,
            RuntimeArtifact::from_source(
                "fetch-contract.js",
                r#"
                globalThis.__nanaFetchContractResult = null;
                globalThis.__nanaFetchContract = {
                  read: () => globalThis.__nanaFetchContractResult,
                };
                (async function () {
                  const headers = new Headers({ "X-Test": " one " });
                  headers.append("x-test", "two");
                  const request = new Request("https://example.test/items", {
                    method: "POST",
                    headers,
                    body: new Uint8Array([1, 2, 3]),
                  });
                  const requestBytes = new Uint8Array(await request.arrayBuffer());
                  let requestSecondRead = "";
                  try { await request.text(); } catch (error) { requestSecondRead = error.name; }

                  const response = new Response('{"ok":true}', {
                    status: 418,
                    statusText: "Teapot",
                    headers: [["content-type", "application/json"]],
                  });
                  const copy = response.clone();
                  const json = await response.json();
                  const copyText = await copy.text();

                  let unsupported = "";
                  try { new Request("https://example.test", { cache: "reload" }); }
                  catch (error) { unsupported = error.name; }

                  const controller = new AbortController();
                  controller.abort();
                  let aborted = "";
                  try { await fetch("https://example.test", { signal: controller.signal }); }
                  catch (error) { aborted = error.name; }

                  globalThis.__nanaFetchContractResult = {
                    header: headers.get("X-Test"),
                    requestBytes: Array.from(requestBytes),
                    requestBodyUsed: request.bodyUsed,
                    requestSecondRead,
                    status: response.status,
                    ok: response.ok,
                    responseBodyUsed: response.bodyUsed,
                    jsonOk: json.ok,
                    copyText,
                    unsupported,
                    aborted,
                  };
                })();
                "#,
            ),
        )
        .unwrap();
        let read = engine.resolve_function("__nanaFetchContract.read").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let result = loop {
            engine.run_microtasks().unwrap();
            let value = engine.invoke(read, &[]).unwrap();
            if let HostValue::Object(_) = value {
                break value;
            }
            assert!(std::time::Instant::now() < deadline);
        };
        let result = result.as_object().unwrap();
        assert_eq!(
            result.get("header").and_then(HostValue::as_str),
            Some("one, two")
        );
        assert_eq!(
            result.get("requestBodyUsed").and_then(HostValue::as_bool),
            Some(true)
        );
        assert_eq!(
            result.get("requestSecondRead").and_then(HostValue::as_str),
            Some("TypeError")
        );
        assert_eq!(
            result.get("status").and_then(HostValue::as_f64),
            Some(418.0)
        );
        assert_eq!(result.get("ok").and_then(HostValue::as_bool), Some(false));
        assert_eq!(
            result.get("responseBodyUsed").and_then(HostValue::as_bool),
            Some(true)
        );
        assert_eq!(
            result.get("jsonOk").and_then(HostValue::as_bool),
            Some(true)
        );
        assert_eq!(
            result.get("copyText").and_then(HostValue::as_str),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            result.get("unsupported").and_then(HostValue::as_str),
            Some("TypeError")
        );
        assert_eq!(
            result.get("aborted").and_then(HostValue::as_str),
            Some("AbortError")
        );
        engine.shutdown();
    }

    #[test]
    fn object_array_options_are_not_stringified_to_object_object() {
        let mut engine = QuickJsEngine::new();
        let mut api = HostApiRegistry::new();
        api.register("capture", |args| {
            Ok(args.first().cloned().unwrap_or(HostValue::Null))
        });
        engine.register_host_api(&api).unwrap();
        engine
            .initialize(RuntimeArtifact::from_source(
                "opts.js",
                r#"
                globalThis.__nanaProbe = {
                  run: () => globalThis.__nanaHost.call('capture', [[
                    { value: 'language', label: '按编程语言' },
                    { value: 'project', label: '按项目' },
                  ]])
                };
                "#,
            ))
            .unwrap();
        let run = engine.resolve_function("__nanaProbe.run").unwrap();
        let result = engine.invoke(run, &[]).unwrap();
        let HostValue::Array(items) = result else {
            panic!("expected array, got {result:?}");
        };
        assert_eq!(items.len(), 2);
        let HostValue::Object(first) = &items[0] else {
            panic!("expected object option, got {:?}", items[0]);
        };
        assert_eq!(
            first.get("value").and_then(HostValue::as_str),
            Some("language")
        );
        assert_eq!(
            first.get("label").and_then(HostValue::as_str),
            Some("按编程语言")
        );
        engine.shutdown();
    }

    #[test]
    fn typed_array_host_arguments_preserve_binary_bytes() {
        let mut engine = QuickJsEngine::new();
        let mut api = HostApiRegistry::new();
        api.register("capture", |args| {
            Ok(args.first().cloned().unwrap_or(HostValue::Null))
        });
        engine.register_host_api(&api).unwrap();
        engine
            .initialize(RuntimeArtifact::from_source(
                "typed-array.js",
                r#"
                globalThis.__nanaProbe = {
                  run: () => globalThis.__nanaHost.call(
                    'capture',
                    [new Uint8Array([0, 127, 255])],
                  )
                };
                "#,
            ))
            .unwrap();
        let run = engine.resolve_function("__nanaProbe.run").unwrap();
        let result = engine.invoke(run, &[]).unwrap();
        assert_eq!(
            result,
            HostValue::Array(vec![
                HostValue::Number(0.0),
                HostValue::Number(127.0),
                HostValue::Number(255.0),
            ])
        );
        engine.shutdown();
    }

    #[test]
    fn runs_shared_vue_runtime_probe() {
        let mut engine = QuickJsEngine::new();
        let (api, state) = probe_host_registry();
        engine.register_host_api(&api).unwrap();
        engine
            .initialize(vue_runtime_probe_artifact())
            .expect("vue probe should initialize on QuickJS");
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
        assert!(
            guard.create_element >= 1,
            "expected createElement host ops, got {guard:?}"
        );
        assert!(guard.insert >= 1, "expected insert host ops, got {guard:?}");
        assert!(
            guard.increment >= 1,
            "expected increment host ops, got {guard:?}"
        );
        assert_eq!(guard.last_count, 2);
        drop(guard);
        engine.shutdown();
    }

    #[test]
    fn compile_and_load_quickjs_bytecode_without_plaintext() {
        let source = r#"
            globalThis.__nanaBytecodeProbe = {
              run: () => ({ ok: true, via: "bytecode", n: 1 + 1 })
            };
        "#;
        let artifact =
            QuickJsEngine::compile_bytecode("probe.qbc.js", source).expect("compile bytecode");
        assert_eq!(artifact.kind, RuntimeArtifactKind::QuickJsBytecode);
        assert!(artifact.is_binary_release());
        // Must not be loadable as UTF-8 source.
        assert!(artifact.source_utf8().is_err());
        // Business identifiers should not appear as contiguous plaintext in typical cases;
        // at minimum the artifact is not valid UTF-8 JS source for initialize_source.
        assert!(!artifact.bytes.is_empty());

        let mut engine = QuickJsEngine::new();
        engine.initialize(artifact).expect("load bytecode");
        let run = engine
            .resolve_function("__nanaBytecodeProbe.run")
            .expect("export");
        let result = engine.invoke(run, &[]).expect("invoke");
        let obj = result.as_object().expect("object");
        assert_eq!(obj.get("ok").and_then(HostValue::as_bool), Some(true));
        assert_eq!(obj.get("n").and_then(HostValue::as_f64), Some(2.0));
        assert_eq!(obj.get("via").and_then(HostValue::as_str), Some("bytecode"));
        engine.shutdown();
    }

    #[test]
    fn runs_phase3_counter_on_real_rust_dom() {
        use nana_js_engine::probe::vue_phase3_artifact;
        use nana_ui_vue::VueHost;

        let mut host = VueHost::with_viewport(800, 600, 1.0);
        let mut engine = QuickJsEngine::new();
        host.attach_engine(&mut engine).unwrap();
        engine.initialize(vue_phase3_artifact()).unwrap();
        host.bind_event_bridge(&mut engine).unwrap();
        let run = engine.resolve_function("__nanaVue.runCounter").unwrap();
        let result = engine.invoke(run, &[]).unwrap();
        engine.run_microtasks().unwrap();
        host.resolve_layout();

        let object = result.as_object().expect("object");
        assert_eq!(object.get("ok").and_then(HostValue::as_bool), Some(true));

        let btn = {
            let doc = host.document();
            let guard = doc.lock().unwrap();
            guard
                .snapshot_boxes()
                .event_targets
                .iter()
                .find(|(_, e)| e == "click")
                .map(|(id, _)| nana_ui_vue::NodeHandle(*id))
                .and_then(|h| guard.layout_box(h))
                .expect("button box")
        };
        assert!(
            host.pointer_click(
                &mut engine,
                btn.x + btn.width / 2.0,
                btn.y + btn.height / 2.0
            )
            .unwrap()
        );
        host.resolve_layout();
        let texts = host.document().lock().unwrap().snapshot_boxes().texts;
        assert!(
            texts.iter().any(|(_, t)| t == "1"),
            "expected count text 1 after click, got {texts:?}"
        );
        engine.shutdown();
    }

    /// Release E2E: compose web-api shim → QuickJS bytecode → VueHost loads
    /// `QuickJsBytecode` (not SourceUtf8) and reproduces Phase 3 counter click.
    #[test]
    fn release_bytecode_compose_shim_runs_phase3_counter() {
        use nana_js_engine::probe::VUE_PHASE3_JS;
        use nana_ui_vue::VueHost;
        use nana_ui_web_api::WEB_API_SHIM_JS;

        let composed = format!("{WEB_API_SHIM_JS}\n{VUE_PHASE3_JS}");
        let artifact = QuickJsEngine::compile_bytecode("vue-phase3.qbc.js", &composed)
            .expect("compile phase3 + shim to bytecode");
        assert_eq!(artifact.kind, RuntimeArtifactKind::QuickJsBytecode);
        assert!(artifact.is_binary_release());
        assert!(artifact.source_utf8().is_err());

        let mut host = VueHost::with_viewport(800, 600, 1.0);
        let mut engine = QuickJsEngine::new();
        host.initialize_with_web_api(&mut engine, artifact)
            .expect("load QuickJsBytecode via VueHost");
        engine.run_microtasks().unwrap();
        host.bind_event_bridge(&mut engine).unwrap();

        let run = engine.resolve_function("__nanaVue.runCounter").unwrap();
        let result = engine.invoke(run, &[]).unwrap();
        engine.run_microtasks().unwrap();
        host.resolve_layout();
        assert_eq!(
            result
                .as_object()
                .and_then(|o| o.get("ok"))
                .and_then(HostValue::as_bool),
            Some(true)
        );

        let btn = {
            let doc = host.document();
            let guard = doc.lock().unwrap();
            guard
                .snapshot_boxes()
                .event_targets
                .iter()
                .find(|(_, e)| e == "click")
                .map(|(id, _)| nana_ui_vue::NodeHandle(*id))
                .and_then(|h| guard.layout_box(h))
                .expect("counter button")
        };
        assert!(
            host.pointer_click(
                &mut engine,
                btn.x + btn.width / 2.0,
                btn.y + btn.height / 2.0
            )
            .unwrap()
        );
        host.resolve_layout();
        let texts = host.document().lock().unwrap().snapshot_boxes().texts;
        assert!(
            texts.iter().any(|(_, t)| t == "1"),
            "bytecode counter click failed, texts={texts:?}"
        );
        engine.shutdown();
    }

    /// web-api: `createElement('template').innerHTML` fills `content.childNodes`
    /// (minimal fragment parser — Markdown sanitize path, not full HTML5).
    #[test]
    fn template_inner_html_parses_fragment_for_sanitize() {
        use nana_js_engine::RuntimeArtifact;
        use nana_ui_vue::VueHost;

        let mut host = VueHost::with_viewport(320, 200, 1.0);
        let mut engine = QuickJsEngine::new();
        host.initialize_with_web_api(
            &mut engine,
            RuntimeArtifact::from_source(
                "template-parse.js",
                r#"
                globalThis.__nanaProbe = {
                  run() {
                    const t = document.createElement("template");
                    t.innerHTML =
                      '<!--skip--><p>Hi <strong>there</strong></p><br><a href="https://x.test">x</a><script>bad</script>';
                    const kids = t.content.childNodes;
                    const tags = Array.from(kids).map((n) =>
                      n.nodeType === Node.ELEMENT_NODE ? n.tagName : n.nodeType
                    );
                    const tagCount = kids.length;
                    const p = kids[0];
                    const strong = p && p.childNodes ? p.childNodes[1] : null;
                    const a = kids[2];
                    const script = kids[3];
                    if (script && script.tagName === "SCRIPT") script.remove();
                    const unknown = document.createElement("template");
                    unknown.innerHTML = '<custom>keep<em>me</em></custom>';
                    const custom = unknown.content.childNodes[0];
                    sanitizeUnwrap(custom);
                    return {
                      tagCount: tagCount,
                      tags: tags,
                      pText: p && p.textContent,
                      strongTag: strong && strong.tagName,
                      href: a && a.getAttribute("href"),
                      afterRemove: t.innerHTML,
                      contentText: t.content.textContent,
                      unwrapped: unknown.innerHTML,
                      commentSkipped: tags.indexOf(8) < 0,
                    };
                    function sanitizeUnwrap(el) {
                      if (!el || el.nodeType !== Node.ELEMENT_NODE) return;
                      for (const child of Array.from(el.childNodes)) sanitizeUnwrap(child);
                      if (String(el.tagName).toLowerCase() === "custom") {
                        el.replaceWith(...Array.from(el.childNodes));
                      }
                    }
                  }
                };
                "#,
            ),
        )
        .unwrap();

        let run = engine.resolve_function("__nanaProbe.run").unwrap();
        let result = engine.invoke(run, &[]).unwrap();
        let obj = result.as_object().expect("probe object");
        assert_eq!(
            obj.get("tagCount").and_then(HostValue::as_f64),
            Some(4.0),
            "expected p/br/a/script (comment skipped)"
        );
        assert_eq!(
            obj.get("commentSkipped").and_then(HostValue::as_bool),
            Some(true)
        );
        assert_eq!(
            obj.get("pText").and_then(HostValue::as_str),
            Some("Hi there")
        );
        assert_eq!(
            obj.get("strongTag").and_then(HostValue::as_str),
            Some("STRONG")
        );
        assert_eq!(
            obj.get("href").and_then(HostValue::as_str),
            Some("https://x.test")
        );
        let after = obj
            .get("afterRemove")
            .and_then(HostValue::as_str)
            .unwrap_or("");
        assert!(
            after.contains("<p>") && after.contains("<strong>there</strong>"),
            "serialize after remove: {after}"
        );
        assert!(
            !after.to_ascii_lowercase().contains("<script"),
            "script must be gone: {after}"
        );
        assert_eq!(
            obj.get("contentText").and_then(HostValue::as_str),
            Some("Hi therex")
        );
        assert_eq!(
            obj.get("unwrapped").and_then(HostValue::as_str),
            Some("keep<em>me</em>")
        );
        engine.shutdown();
    }

    /// Integration: applications may add business APIs, but cannot replace a
    /// framework renderer API; the compatibility shim does not create Tauri globals.
    #[test]
    fn vue_host_merges_application_api_without_tauri_globals() {
        use nana_js_engine::{HostApiRegistry, RuntimeArtifact};
        use nana_ui_vue::VueHost;

        let mut host = VueHost::with_viewport(320, 200, 1.0);
        let mut application_api = HostApiRegistry::new();
        application_api.register("loadRepositories", |args| {
            Ok(HostValue::String(
                args.first()
                    .and_then(HostValue::as_str)
                    .unwrap_or("anonymous")
                    .to_string(),
            ))
        });
        let mut engine = QuickJsEngine::new();
        host.initialize_with_web_api_and_host_api(
            &mut engine,
            RuntimeArtifact::from_source(
                "application-api-probe.js",
                r#"
                  globalThis.__nanaApplicationProbe = {
                    run() {
                      return {
                        value: globalThis.__nanaHost.call("loadRepositories", ["octocat"]),
                        hasTauri: "__TAURI_INTERNALS__" in globalThis ||
                          "__TAURI__" in globalThis ||
                          "__TAURI_INTERNALS__" in globalThis.window ||
                          "__TAURI__" in globalThis.window,
                      };
                    }
                  };
                "#,
            ),
            &application_api,
        )
        .unwrap();

        let run = engine
            .resolve_function("__nanaApplicationProbe.run")
            .unwrap();
        let result = engine.invoke(run, &[]).unwrap();
        let result = result.as_object().expect("application result");
        assert_eq!(
            result.get("value").and_then(HostValue::as_str),
            Some("octocat")
        );
        assert_eq!(
            result.get("hasTauri").and_then(HostValue::as_bool),
            Some(false)
        );
        engine.shutdown();

        let mut collision = HostApiRegistry::new();
        collision.register("createElement", |_| Ok(HostValue::Null));
        let mut engine = QuickJsEngine::new();
        let error = host
            .initialize_with_web_api_and_host_api(
                &mut engine,
                RuntimeArtifact::from_source("collision.js", ""),
                &collision,
            )
            .unwrap_err();
        assert_eq!(error.message, "duplicate host API name `createElement`");
    }

    #[test]
    fn vue_sfc_fetch_updates_semantic_tree_on_quickjs() {
        use std::time::{Duration, Instant};

        use nana_ui_vue::{MountOptions, mount_vue_as_nana};
        use nana_ui_web_api::{
            FetchError, FetchHost, FetchPolicy, FetchRequest, FetchResponse, shared_fetch_host,
        };

        #[derive(Debug)]
        struct FixtureFetch {
            policy: FetchPolicy,
        }

        impl FetchHost for FixtureFetch {
            fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
                assert_eq!(request.url, "/fixture");
                assert!(
                    request
                        .headers
                        .iter()
                        .any(|(name, value)| name == "x-nana-fixture" && value == "sfc")
                );
                let body = br#"{"message":"ready","sequence":7}"#.to_vec();
                Ok(FetchResponse {
                    url: request.url,
                    status: 200,
                    status_text: "OK".into(),
                    headers: vec![("content-type".into(), "application/json".into())],
                    body,
                    redirected: false,
                })
            }

            fn policy(&self) -> &FetchPolicy {
                &self.policy
            }
        }

        let mut host = mount_vue_as_nana(MountOptions {
            width: 480,
            height: 320,
            fetch_host: Some(shared_fetch_host(FixtureFetch {
                policy: FetchPolicy::default(),
            })),
            ..MountOptions::default()
        });
        host.inject_stylesheet(VUE_SFC_COMPAT_CSS);
        let mut engine = QuickJsEngine::new();
        let mut application_api = HostApiRegistry::new();
        application_api.register("fixtureApplicationApi", |_| {
            Ok(HostValue::string("quickjs"))
        });
        host.initialize_with_web_api_and_host_api(
            &mut engine,
            vue_sfc_compat_artifact(),
            &application_api,
        )
        .unwrap();
        host.bind_event_bridge(&mut engine).unwrap();
        let probe = engine.resolve_function("__nanaSfcFixture.probe").unwrap();
        let probe = engine.invoke(probe, &[]).unwrap();
        let probe = probe.as_object().unwrap();
        assert_eq!(
            probe.get("applicationValue").and_then(HostValue::as_str),
            Some("quickjs")
        );
        assert_eq!(
            probe.get("hasTauri").and_then(HostValue::as_bool),
            Some(false)
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            engine.run_microtasks().unwrap();
            host.pump_frame(&mut engine).unwrap();
            let snapshot = host.semantic_snapshot();
            let labels = snapshot
                .widgets
                .iter()
                .map(|widget| widget.props.label.as_str())
                .collect::<Vec<_>>();
            if labels.contains(&"ready:7") {
                assert!(labels.contains(&"32"));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "SFC fetch did not update: {labels:?}"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        engine.shutdown();
    }

    #[test]
    fn resize_observer_reads_layout_box_and_notifies_on_layout_pump() {
        use nana_ui_web_api::{WEB_API_SHIM_JS, register_web_api_host_ops, shared_web_api_state};
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        let sizes = Arc::new(Mutex::new(HashMap::<u64, (f64, f64)>::new()));
        sizes.lock().unwrap().insert(7, (320.0, 48.0));

        let mut api = HostApiRegistry::new();
        register_web_api_host_ops(&mut api, shared_web_api_state());
        {
            let sizes = Arc::clone(&sizes);
            api.register("layoutBox", move |args| {
                let nid = args.first().and_then(HostValue::as_f64).unwrap_or(0.0) as u64;
                let (w, h) = sizes
                    .lock()
                    .unwrap()
                    .get(&nid)
                    .copied()
                    .unwrap_or((0.0, 0.0));
                Ok(HostValue::Object(
                    [
                        ("x".into(), HostValue::Number(0.0)),
                        ("y".into(), HostValue::Number(0.0)),
                        ("width".into(), HostValue::Number(w)),
                        ("height".into(), HostValue::Number(h)),
                        ("top".into(), HostValue::Number(0.0)),
                        ("left".into(), HostValue::Number(0.0)),
                        ("bottom".into(), HostValue::Number(h)),
                        ("right".into(), HostValue::Number(w)),
                    ]
                    .into_iter()
                    .collect(),
                ))
            });
        }

        let mut engine = QuickJsEngine::new();
        engine.register_host_api(&api).unwrap();
        let source = format!(
            "{WEB_API_SHIM_JS}\n{}",
            r#"
            globalThis.__roSeen = [];
            globalThis.__roSkipCalled = false;
            (function () {
              const el = { __nid: 7 };
              const ro = new ResizeObserver(function (entries) {
                const e = entries[0];
                globalThis.__roSeen.push({
                  w: e.contentRect.width,
                  h: e.contentRect.height,
                  bw: e.borderBoxSize[0].inlineSize,
                  bh: e.borderBoxSize[0].blockSize,
                });
              });
              ro.observe(el);
              const bare = {};
              new ResizeObserver(function () {
                globalThis.__roSkipCalled = true;
              }).observe(bare);
            })();
            globalThis.__roProbe = {
              snapshot() {
                return {
                  seen: globalThis.__roSeen.slice(),
                  skipCalled: globalThis.__roSkipCalled,
                };
              },
              notify() {
                return globalThis.__nanaNotifyLayout();
              },
            };
            "#
        );
        engine
            .initialize(RuntimeArtifact::from_source("resize-observer.js", source))
            .unwrap();
        engine.run_microtasks().unwrap();

        let snap = engine.resolve_function("__roProbe.snapshot").unwrap();
        let first = engine.invoke(snap, &[]).unwrap();
        let first_obj = first.as_object().expect("snapshot object");
        assert_eq!(
            first_obj.get("skipCalled").and_then(HostValue::as_bool),
            Some(false),
            "targets without layoutBox/__nid must be skipped"
        );
        let HostValue::Array(seen) = first_obj.get("seen").cloned().unwrap_or(HostValue::Null)
        else {
            panic!("expected seen array");
        };
        assert_eq!(seen.len(), 1, "initial observe should deliver once");
        let HostValue::Object(entry) = &seen[0] else {
            panic!("expected entry object");
        };
        assert_eq!(entry.get("w").and_then(HostValue::as_f64), Some(320.0));
        assert_eq!(entry.get("h").and_then(HostValue::as_f64), Some(48.0));
        assert_eq!(entry.get("bw").and_then(HostValue::as_f64), Some(320.0));
        assert_eq!(entry.get("bh").and_then(HostValue::as_f64), Some(48.0));
        assert_ne!(entry.get("w").and_then(HostValue::as_f64), Some(220.0));
        assert_ne!(entry.get("h").and_then(HostValue::as_f64), Some(640.0));

        sizes.lock().unwrap().insert(7, (400.0, 120.0));
        let notify = engine.resolve_function("__roProbe.notify").unwrap();
        engine.invoke(notify, &[]).unwrap();
        engine.run_microtasks().unwrap();

        let second = engine.invoke(snap, &[]).unwrap();
        let second_obj = second.as_object().expect("snapshot object");
        let HostValue::Array(seen2) = second_obj.get("seen").cloned().unwrap_or(HostValue::Null)
        else {
            panic!("expected seen array after notify");
        };
        assert_eq!(
            seen2.len(),
            2,
            "layout notify must redeliver on size change"
        );
        let HostValue::Object(entry2) = &seen2[1] else {
            panic!("expected second entry");
        };
        assert_eq!(entry2.get("w").and_then(HostValue::as_f64), Some(400.0));
        assert_eq!(entry2.get("h").and_then(HostValue::as_f64), Some(120.0));

        engine.shutdown();
    }

    /// Host pumps focus/blur/resize/visibilitychange into shim EventTarget
    /// (focus refresh + Page Visibility listeners).
    #[test]
    fn vue_host_pumps_window_lifecycle_events() {
        use nana_js_engine::RuntimeArtifact;
        use nana_ui_vue::{VueHost, WindowLifecycleEvent};
        use nana_ui_web_api::WEB_API_SHIM_JS;

        let source = format!(
            "{WEB_API_SHIM_JS}\n{}",
            r#"
            globalThis.__nanaFireEvent = function () { return true; };
            globalThis.__lifecycleLog = [];
            window.addEventListener("focus", function () {
              globalThis.__lifecycleLog.push("focus:" + String(document.hasFocus()));
            });
            window.addEventListener("blur", function () {
              globalThis.__lifecycleLog.push("blur:" + String(document.hasFocus()));
            });
            window.addEventListener("resize", function () {
              globalThis.__lifecycleLog.push(
                "resize:" + String(window.innerWidth) + "x" + String(window.innerHeight)
              );
            });
            document.addEventListener("visibilitychange", function () {
              globalThis.__lifecycleLog.push(
                "visibility:" + String(document.visibilityState) + ":" + String(document.hidden)
              );
            });
            globalThis.__lifecycleSnapshot = function () {
              return {
                log: globalThis.__lifecycleLog.slice(),
                focused: document.hasFocus(),
                visibility: document.visibilityState,
                hidden: document.hidden,
                w: window.innerWidth,
                h: window.innerHeight,
              };
            };
            "#
        );

        let mut host = VueHost::with_viewport(800, 600, 1.0);
        let mut engine = QuickJsEngine::new();
        host.initialize_with_web_api(
            &mut engine,
            RuntimeArtifact::from_source("lifecycle-pump.js", source),
        )
        .expect("shim + probe");
        host.bind_event_bridge(&mut engine).expect("bind");

        assert!(
            host.pump_lifecycle(&mut engine, WindowLifecycleEvent::Blur)
                .expect("blur"),
            "blur must reach shim"
        );
        assert!(
            host.pump_lifecycle(&mut engine, WindowLifecycleEvent::Focus)
                .expect("focus")
        );
        assert!(
            host.pump_lifecycle(
                &mut engine,
                WindowLifecycleEvent::Resize {
                    width: 1024.0,
                    height: 768.0,
                },
            )
            .expect("resize")
        );
        assert!(
            host.pump_lifecycle(
                &mut engine,
                WindowLifecycleEvent::VisibilityChange { hidden: true },
            )
            .expect("visibility hidden")
        );
        assert!(
            host.pump_lifecycle(
                &mut engine,
                WindowLifecycleEvent::VisibilityChange { hidden: false },
            )
            .expect("visibility visible")
        );

        let probe = engine
            .resolve_function("__lifecycleSnapshot")
            .expect("__lifecycleSnapshot");
        let result = engine.invoke(probe, &[]).expect("snapshot");
        let obj = result.as_object().expect("object");
        let HostValue::Array(log) = obj.get("log").cloned().unwrap_or(HostValue::Null) else {
            panic!("expected log array");
        };
        let entries: Vec<String> = log
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        assert!(
            entries.iter().any(|e| e.starts_with("blur:")),
            "expected blur entry, got {entries:?}"
        );
        assert!(
            entries.iter().any(|e| e.starts_with("focus:")),
            "expected focus entry, got {entries:?}"
        );
        assert!(
            entries.iter().any(|e| e == "resize:1024x768"),
            "expected resize entry, got {entries:?}"
        );
        assert!(
            entries.iter().any(|e| e == "visibility:hidden:true"),
            "expected hidden visibility, got {entries:?}"
        );
        assert!(
            entries.iter().any(|e| e == "visibility:visible:false"),
            "expected visible again, got {entries:?}"
        );
        assert_eq!(obj.get("focused").and_then(HostValue::as_bool), Some(true));
        assert_eq!(
            obj.get("visibility").and_then(HostValue::as_str),
            Some("visible")
        );
        assert_eq!(obj.get("w").and_then(HostValue::as_f64), Some(1024.0));
        assert_eq!(obj.get("h").and_then(HostValue::as_f64), Some(768.0));

        engine.shutdown();
    }
}
