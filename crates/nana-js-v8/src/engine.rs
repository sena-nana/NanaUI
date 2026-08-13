//! Real V8 `JsEngine` implementation (feature = "engine").
//!
//! Bound to crates.io `v8 = "150.4.0"` (rusty_v8 successor package name).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, Once};

use nana_js_engine::{
    HostApiRegistry, HostCancellationToken, HostEventSender, HostInvocation, HostPendingCall,
    HostRequestContext, HostRequestId, HostResourceHandle, HostResourceRegistry, HostValue,
    JsDiagnosticEvent, JsDiagnosticLevel, JsDiagnosticSink, JsEngine, JsEngineError, JsException,
    JsFunctionId, RuntimeArtifact,
};

struct HostApiSlot {
    api: Mutex<HostApiRegistry>,
    state: Mutex<HostBridgeState>,
    resources: HostResourceRegistry,
    diagnostics: Mutex<Option<JsDiagnosticSink>>,
}

#[derive(Default)]
struct ResourceFinalizerSlot {
    handles: RefCell<Vec<v8::Weak<v8::Object>>>,
}

#[derive(Default)]
struct HostBridgeState {
    next_request_id: u64,
    pending: BTreeMap<u64, PendingHostRequest>,
}

enum PendingHostRequest {
    Ready(Result<HostValue, JsException>),
    Waiting(HostPendingCall),
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
    inspector_session: Option<v8::inspector::V8InspectorSession>,
    inspector: Option<v8::inspector::V8Inspector>,
    inspector_transport: Option<V8InspectorTransport>,
    isolate: Option<v8::OwnedIsolate>,
    context: Option<v8::Global<v8::Context>>,
    host_api: HostApiRegistry,
    functions: BTreeMap<u64, String>,
    next_function_id: u64,
    resources: HostResourceRegistry,
    events: HostEventSender,
    diagnostics: Option<JsDiagnosticSink>,
    shut_down: bool,
}

/// In-process Chrome DevTools Protocol transport. A development server can
/// forward messages from this queue to any CDP frontend without exposing a
/// network listener from NanaUI itself.
#[derive(Debug, Clone, Default)]
pub struct V8InspectorTransport {
    messages: Arc<Mutex<VecDeque<String>>>,
}

impl V8InspectorTransport {
    pub fn drain_messages(&self) -> Vec<String> {
        let Ok(mut messages) = self.messages.lock() else {
            return Vec::new();
        };
        messages.drain(..).collect()
    }
}

struct InspectorClient;

impl v8::inspector::V8InspectorClientImpl for InspectorClient {}

struct InspectorChannel {
    messages: Arc<Mutex<VecDeque<String>>>,
}

impl InspectorChannel {
    fn push(&self, mut message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        let Some(message) = message.as_mut() else {
            return;
        };
        if let Ok(mut messages) = self.messages.lock() {
            messages.push_back(message.string().to_string());
        }
    }
}

impl v8::inspector::ChannelImpl for InspectorChannel {
    fn send_response(&self, _call_id: i32, message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        self.push(message);
    }

    fn send_notification(&self, message: v8::UniquePtr<v8::inspector::StringBuffer>) {
        self.push(message);
    }

    fn flush_protocol_notifications(&self) {}
}

impl Default for V8Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl V8Engine {
    pub fn new() -> Self {
        Self {
            inspector_session: None,
            inspector: None,
            inspector_transport: None,
            isolate: None,
            context: None,
            host_api: HostApiRegistry::new(),
            functions: BTreeMap::new(),
            next_function_id: 1,
            resources: HostResourceRegistry::new(),
            events: HostEventSender::default(),
            diagnostics: None,
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
            isolate.set_capture_stack_trace_for_uncaught_exceptions(true, 64);
            isolate.add_message_listener(v8_message_callback);
            isolate.set_promise_reject_callback(v8_promise_reject_callback);
            isolate.set_slot(HostApiSlot {
                api: Mutex::new(self.host_api.clone()),
                state: Mutex::new(HostBridgeState::default()),
                resources: self.resources.clone(),
                diagnostics: Mutex::new(self.diagnostics.clone()),
            });
            isolate.set_slot(ResourceFinalizerSlot::default());

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
        self.inspector_session = None;
        self.inspector = None;
        self.inspector_transport = None;
        self.context = None;
        self.isolate = None;
        if *live > 0 {
            *live -= 1;
        }
        gate.cv.notify_all();
    }

    /// Connect structured runtime diagnostics before or after initialization.
    pub fn set_diagnostic_sink(&mut self, sink: Option<JsDiagnosticSink>) {
        self.diagnostics = sink.clone();
        if let Some(isolate) = self.isolate.as_mut() {
            if let Some(slot) = isolate.get_slot::<HostApiSlot>() {
                if let Ok(mut current) = slot.diagnostics.lock() {
                    *current = sink;
                }
            }
        }
    }

    /// Start an in-process V8 inspector session in the existing isolate.
    /// NanaUI does not open a port; development tooling owns any CDP transport.
    pub fn enable_inspector(&mut self) -> Result<V8InspectorTransport, JsEngineError> {
        self.ensure_isolate()?;
        if let Some(transport) = &self.inspector_transport {
            return Ok(transport.clone());
        }

        let transport = V8InspectorTransport::default();
        let client = v8::inspector::V8InspectorClient::new(Box::new(InspectorClient));
        let isolate = self
            .isolate
            .as_mut()
            .ok_or_else(|| JsEngineError::new("V8 isolate missing"))?;
        let inspector = v8::inspector::V8Inspector::create(isolate, client);
        {
            v8::scope!(let scope, isolate);
            let context = v8::Local::new(
                scope,
                self.context
                    .as_ref()
                    .ok_or_else(|| JsEngineError::new("V8 context missing"))?,
            );
            inspector.context_created(
                context,
                1,
                v8::inspector::StringView::from(&b"NanaUI"[..]),
                v8::inspector::StringView::empty(),
            );
        }
        let channel = v8::inspector::Channel::new(Box::new(InspectorChannel {
            messages: Arc::clone(&transport.messages),
        }));
        let session = inspector.connect(
            1,
            channel,
            v8::inspector::StringView::empty(),
            v8::inspector::V8InspectorClientTrustLevel::FullyTrusted,
        );
        self.inspector = Some(inspector);
        self.inspector_session = Some(session);
        self.inspector_transport = Some(transport.clone());
        Ok(transport)
    }

    /// Send one Chrome DevTools Protocol JSON message to V8.
    pub fn dispatch_inspector_protocol_message(
        &mut self,
        message: &str,
    ) -> Result<(), JsEngineError> {
        if self.inspector_session.is_none() {
            self.enable_inspector()?;
        }
        let session = self
            .inspector_session
            .as_ref()
            .expect("inspector session initialized");
        let isolate = self
            .isolate
            .as_mut()
            .ok_or_else(|| JsEngineError::new("V8 isolate missing"))?;
        v8::scope!(let scope, isolate);
        let context = v8::Local::new(
            scope,
            self.context
                .as_ref()
                .ok_or_else(|| JsEngineError::new("V8 context missing"))?,
        );
        let _scope = &mut v8::ContextScope::new(scope, context);
        session.dispatch_protocol_message(v8::inspector::StringView::from(message.as_bytes()));
        Ok(())
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
            *slot
                .diagnostics
                .lock()
                .map_err(|_| JsEngineError::new("diagnostic sink slot poisoned"))? =
                self.diagnostics.clone();
        } else {
            isolate.set_slot(HostApiSlot {
                api: Mutex::new(self.host_api.clone()),
                state: Mutex::new(HostBridgeState::default()),
                resources: self.resources.clone(),
                diagnostics: Mutex::new(self.diagnostics.clone()),
            });
        }
        if isolate.get_slot::<ResourceFinalizerSlot>().is_none() {
            isolate.set_slot(ResourceFinalizerSlot::default());
        }
        Ok(())
    }

    fn install_shims_and_host(&mut self) -> Result<(), JsEngineError> {
        self.sync_host_slot()?;
        with_context(self, |scope| {
            install_host_bridge(scope)?;
            eval_script(
                scope,
                r##"
                if (typeof globalThis.process === "undefined") {
                  globalThis.process = { env: { NODE_ENV: "production" } };
                }
                if (typeof globalThis.console === "undefined") {
                  globalThis.console = { log() {}, warn() {}, error() {}, info() {}, debug() {} };
                }
                if (typeof globalThis.queueMicrotask !== "function") {
                  globalThis.queueMicrotask = function (fn) { Promise.resolve().then(fn); };
                }
                "##,
            )?;
            Ok(())
        })
    }

    fn drain_host_bridge(&mut self) -> Result<(), JsEngineError> {
        let completions = {
            let isolate = self
                .isolate
                .as_mut()
                .ok_or_else(|| JsEngineError::new("V8 isolate missing"))?;
            let Some(slot) = isolate.get_slot::<HostApiSlot>() else {
                return Err(JsEngineError::new("host api slot missing"));
            };
            let mut state = slot
                .state
                .lock()
                .map_err(|_| JsEngineError::new("host bridge state poisoned"))?;
            let mut completed = Vec::new();
            let mut completed_ids = Vec::new();
            for (&id, request) in &state.pending {
                let result = match request {
                    PendingHostRequest::Ready(result) => Some(result.clone()),
                    PendingHostRequest::Waiting(pending) => pending.try_take(),
                };
                if let Some(result) = result {
                    completed_ids.push(id);
                    completed.push((id, result));
                }
            }
            for id in completed_ids {
                state.pending.remove(&id);
            }
            completed
        };
        let events = self.events.drain();
        if completions.is_empty() && events.is_empty() {
            return Ok(());
        }

        with_context(self, |scope| {
            if !completions.is_empty() {
                let settle = lookup_function(scope, "__nanaHostSettle")?;
                for (id, result) in completions {
                    let (ok, value) = match result {
                        Ok(value) => (true, value),
                        Err(exception) => (false, exception.to_host_value()),
                    };
                    call_function(
                        scope,
                        settle,
                        &[
                            HostValue::String(id.to_string()),
                            HostValue::Bool(ok),
                            value,
                        ],
                    )?;
                }
            }
            if !events.is_empty() {
                let emit = lookup_function(scope, "__nanaHostEmit")?;
                for event in events {
                    call_function(scope, emit, &[HostValue::String(event.name), event.payload])?;
                }
            }
            Ok(())
        })
    }

    fn cancel_pending_host_requests(&mut self) {
        let Some(isolate) = self.isolate.as_mut() else {
            return;
        };
        let Some(slot) = isolate.get_slot::<HostApiSlot>() else {
            return;
        };
        if let Ok(mut state) = slot.state.lock() {
            for request in state.pending.values() {
                if let PendingHostRequest::Waiting(pending) = request {
                    pending.cancel();
                }
            }
            state.pending.clear();
        }
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
        self.drain_host_bridge()?;
        if let Some(isolate) = self.isolate.as_mut() {
            isolate.perform_microtask_checkpoint();
            if let Some(finalizers) = isolate.get_slot::<ResourceFinalizerSlot>() {
                finalizers
                    .handles
                    .borrow_mut()
                    .retain(|handle| !handle.is_empty());
            }
        }
        Ok(())
    }

    fn host_event_sender(&self) -> Option<HostEventSender> {
        Some(self.events.clone())
    }

    fn host_resources(&self) -> Option<HostResourceRegistry> {
        Some(self.resources.clone())
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
        self.cancel_pending_host_requests();
        self.resources.clear();
        self.events.clear();
        self.functions.clear();
        self.drop_isolate();
        self.shut_down = true;
    }
}

impl Drop for V8Engine {
    fn drop(&mut self) {
        self.cancel_pending_host_requests();
        self.resources.clear();
        self.events.clear();
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

extern "C" fn v8_message_callback<'s>(
    message: v8::Local<'s, v8::Message>,
    exception: v8::Local<'s, v8::Value>,
) {
    v8::callback_scope!(unsafe scope, message);
    v8::scope!(let scope, scope);
    let sink = diagnostic_sink(scope);
    let Some(sink) = sink else {
        return;
    };
    let fallback = message.get(scope).to_rust_string_lossy(scope);
    let (exception_message, stack) = diagnostic_exception(scope, exception);
    sink(JsDiagnosticEvent {
        source: "v8.exception".into(),
        level: JsDiagnosticLevel::Error,
        message: if exception_message.is_empty() {
            fallback
        } else {
            exception_message
        },
        stack,
    });
}

extern "C" fn v8_promise_reject_callback(message: v8::PromiseRejectMessage) {
    if message.get_event() != v8::PromiseRejectEvent::PromiseRejectWithNoHandler {
        return;
    }
    v8::callback_scope!(unsafe scope, &message);
    v8::scope!(let scope, scope);
    let sink = diagnostic_sink(scope);
    let Some(sink) = sink else {
        return;
    };
    let (message, stack) = message
        .get_value()
        .map(|value| diagnostic_exception(scope, value))
        .unwrap_or_else(|| ("Promise rejected without a value".into(), None));
    sink(JsDiagnosticEvent {
        source: "v8.promise".into(),
        level: JsDiagnosticLevel::Error,
        message,
        stack,
    });
}

fn diagnostic_sink(scope: &mut v8::PinScope) -> Option<JsDiagnosticSink> {
    scope
        .get_slot::<HostApiSlot>()
        .and_then(|slot| slot.diagnostics.lock().ok()?.clone())
}

fn diagnostic_exception(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> (String, Option<String>) {
    let message = value.to_rust_string_lossy(scope);
    let stack = value.to_object(scope).and_then(|object| {
        let key = v8::String::new(scope, "stack")?;
        let stack = object.get(scope, key.into())?;
        (!stack.is_null_or_undefined()).then(|| stack.to_rust_string_lossy(scope))
    });
    (message, stack)
}

fn install_host_bridge(scope: &mut v8::PinScope) -> Result<(), JsEngineError> {
    let global = scope.get_current_context().global(scope);
    install_native_function(scope, global, "__nanaHostRaw", host_raw_callback)?;
    install_native_function(
        scope,
        global,
        "__nanaHostCallRaw",
        host_call_direct_callback,
    )?;
    install_native_function(scope, global, "__nanaHostInvokeRaw", host_invoke_callback)?;
    install_native_function(scope, global, "__nanaHostCancelRaw", host_cancel_callback)?;
    install_native_function(
        scope,
        global,
        "__nanaHostReleaseResourceRaw",
        host_release_resource_callback,
    )?;

    eval_script(scope, nana_js_engine::HOST_BRIDGE_INSTALL_JS)?;
    Ok(())
}

fn install_native_function(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
    name: &str,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Result<(), JsEngineError> {
    let key = v8::String::new(scope, name)
        .ok_or_else(|| JsEngineError::new(format!("failed to allocate {name} string")))?;
    let func = v8::Function::new(scope, callback)
        .ok_or_else(|| JsEngineError::new(format!("failed to create {name} function")))?;
    if global.set(scope, key.into(), func.into()).is_none() {
        return Err(JsEngineError::new(format!(
            "failed to install native function {name}"
        )));
    }
    Ok(())
}

fn host_call_direct_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = v8_string(scope, args.get(0));
    let host_args = match v8_to_host(scope, args.get(1)).map(normalize_host_args) {
        Ok(args) => args,
        Err(error) => {
            throw_host_error(scope, &error.message);
            return;
        }
    };
    let result = match scope.get_slot::<HostApiSlot>() {
        Some(slot) => match slot.api.lock() {
            Ok(api) => api.call(&name, &host_args),
            Err(_) => Err(JsException::new("host api slot poisoned")),
        },
        None => Err(JsException::new("host api slot missing")),
    };
    match result {
        Ok(value) => match host_to_v8(scope, &value) {
            Ok(value) => rv.set(value),
            Err(error) => throw_host_error(scope, &error.message),
        },
        Err(exception) => throw_host_exception(scope, &exception),
    }
}

fn host_invoke_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = v8_string(scope, args.get(0));
    let host_args = match v8_to_host(scope, args.get(1)).map(normalize_host_args) {
        Ok(args) => args,
        Err(error) => {
            throw_host_error(scope, &error.message);
            return;
        }
    };
    let Some(slot) = scope.get_slot::<HostApiSlot>() else {
        throw_host_error(scope, "host api slot missing");
        return;
    };
    let id = {
        let Ok(mut state) = slot.state.lock() else {
            throw_host_error(scope, "host bridge state poisoned");
            return;
        };
        state.next_request_id = state.next_request_id.saturating_add(1).max(1);
        state.next_request_id
    };
    let cancellation = HostCancellationToken::default();
    let context = HostRequestContext::new(HostRequestId(id), cancellation, slot.resources.clone());
    let invocation = match slot.api.lock() {
        Ok(api) => api.invoke(&name, host_args, context),
        Err(_) => HostInvocation::Ready(Err(JsException::new("host api slot poisoned"))),
    };
    let request = match invocation {
        HostInvocation::Ready(result) => PendingHostRequest::Ready(result),
        HostInvocation::Pending(pending) => PendingHostRequest::Waiting(pending),
    };
    let Ok(mut state) = slot.state.lock() else {
        throw_host_error(scope, "host bridge state poisoned");
        return;
    };
    state.pending.insert(id, request);
    if let Some(id) = v8::String::new(scope, &id.to_string()) {
        rv.set(id.into());
    }
}

fn host_cancel_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let id = v8_string(scope, args.get(0)).parse::<u64>().ok();
    let cancelled = id.is_some_and(|id| {
        let Some(slot) = scope.get_slot::<HostApiSlot>() else {
            return false;
        };
        let Ok(mut state) = slot.state.lock() else {
            return false;
        };
        let Some(request) = state.pending.remove(&id) else {
            return false;
        };
        if let PendingHostRequest::Waiting(pending) = request {
            pending.cancel();
        }
        true
    });
    rv.set(v8::Boolean::new(scope, cancelled).into());
}

fn host_release_resource_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let handle = v8_to_host(scope, args.get(0))
        .ok()
        .and_then(|value| value.as_resource().cloned());
    let released = handle.is_some_and(|handle| {
        scope
            .get_slot::<HostApiSlot>()
            .is_some_and(|slot| slot.resources.release(&handle))
    });
    rv.set(v8::Boolean::new(scope, released).into());
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

fn lookup_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Function>, JsEngineError> {
    v8::Local::<v8::Function>::try_from(lookup_path(scope, name)?)
        .map_err(|_| JsEngineError::new(format!("`{name}` is not a function")))
}

fn call_function(
    scope: &mut v8::PinScope,
    function: v8::Local<v8::Function>,
    args: &[HostValue],
) -> Result<(), JsEngineError> {
    let recv = v8::undefined(scope).into();
    let js_args = args
        .iter()
        .map(|arg| host_to_v8(scope, arg))
        .collect::<Result<Vec<_>, _>>()?;
    v8::tc_scope!(let try_catch, scope);
    function
        .call(try_catch, recv, &js_args)
        .map(|_| ())
        .ok_or_else(|| exception_from_try_catch(try_catch))
}

fn host_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &HostValue,
) -> Result<v8::Local<'s, v8::Value>, JsEngineError> {
    match value {
        HostValue::Null => Ok(v8::null(scope).into()),
        HostValue::Undefined => Ok(v8::undefined(scope).into()),
        HostValue::Bool(value) => Ok(v8::Boolean::new(scope, *value).into()),
        HostValue::Number(value) => Ok(v8::Number::new(scope, *value).into()),
        HostValue::BigInt(value) => Ok(v8::BigInt::new_from_u64(scope, *value).into()),
        HostValue::String(value) => v8::String::new(scope, value)
            .map(Into::into)
            .ok_or_else(|| JsEngineError::new("failed to allocate host string")),
        HostValue::Bytes(bytes) => {
            let backing = v8::ArrayBuffer::new_backing_store(scope, bytes.len());
            for (target, source) in backing.iter().zip(bytes) {
                target.set(*source);
            }
            let backing = backing.make_shared();
            Ok(v8::ArrayBuffer::with_backing_store(scope, &backing).into())
        }
        HostValue::Array(items) => {
            let array = v8::Array::new(scope, items.len() as i32);
            for (index, item) in items.iter().enumerate() {
                let item = host_to_v8(scope, item)?;
                if array.set_index(scope, index as u32, item).is_none() {
                    return Err(JsEngineError::new("failed to populate host array"));
                }
            }
            Ok(array.into())
        }
        HostValue::Object(entries) => {
            let object = v8::Object::new(scope);
            for (key, value) in entries {
                set_object_property(scope, object, key, value)?;
            }
            Ok(object.into())
        }
        HostValue::Resource(handle) => host_resource_to_v8(scope, handle),
        HostValue::Function(id) => {
            let object = v8::Object::new(scope);
            set_object_property(scope, object, "__fn", &HostValue::Number(id.0 as f64))?;
            Ok(object.into())
        }
        HostValue::ObjectRef(id) => {
            let object = v8::Object::new(scope);
            set_object_property(scope, object, "__obj", &HostValue::Number(id.0 as f64))?;
            Ok(object.into())
        }
    }
}

fn v8_to_host(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<HostValue, JsEngineError> {
    v8_to_host_inner(scope, value, 0, &mut HashSet::new())
}

fn v8_to_host_inner(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    depth: usize,
    seen: &mut HashSet<i32>,
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
    if value.is_big_int() {
        let bigint = v8::Local::<v8::BigInt>::try_from(value)
            .map_err(|_| JsEngineError::new("invalid V8 BigInt"))?;
        let (value, lossless) = bigint.u64_value();
        return if lossless {
            Ok(HostValue::BigInt(value))
        } else {
            Err(JsEngineError::new(
                "negative or oversized BigInt cannot cross the Nana host bridge",
            ))
        };
    }
    if value.is_string() {
        let s = value
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default();
        return Ok(HostValue::String(s));
    }
    if value.is_array_buffer() {
        let buffer = v8::Local::<v8::ArrayBuffer>::try_from(value)
            .map_err(|_| JsEngineError::new("invalid V8 ArrayBuffer"))?;
        let backing = buffer.get_backing_store();
        return Ok(HostValue::Bytes(
            backing.iter().map(std::cell::Cell::get).collect(),
        ));
    }
    if value.is_array_buffer_view() {
        let view = v8::Local::<v8::ArrayBufferView>::try_from(value)
            .map_err(|_| JsEngineError::new("invalid V8 ArrayBufferView"))?;
        let mut bytes = vec![0; view.byte_length()];
        let copied = view.copy_contents(&mut bytes);
        bytes.truncate(copied);
        return Ok(HostValue::Bytes(bytes));
    }
    if value.is_array() {
        if depth >= 8 {
            return Ok(HostValue::Undefined);
        }
        let array = v8::Local::<v8::Array>::try_from(value)
            .map_err(|_| JsEngineError::new("invalid V8 Array"))?;
        let mut values = Vec::with_capacity(array.length() as usize);
        for index in 0..array.length() {
            let item = array
                .get_index(scope, index)
                .unwrap_or_else(|| v8::undefined(scope).into());
            values.push(v8_to_host_inner(scope, item, depth + 1, seen)?);
        }
        return Ok(HostValue::Array(values));
    }
    if value.is_function() || value.is_symbol() {
        return Ok(HostValue::Undefined);
    }
    if value.is_object() {
        if depth >= 8 {
            return Ok(HostValue::Undefined);
        }
        let object = value
            .to_object(scope)
            .ok_or_else(|| JsEngineError::new("failed to convert V8 object"))?;
        if let Some(handle) = host_resource_from_v8(scope, object) {
            return Ok(HostValue::Resource(handle));
        }
        let identity = object.get_identity_hash().get();
        if !seen.insert(identity) {
            return Ok(HostValue::Undefined);
        }
        let names = object
            .get_own_property_names(scope, Default::default())
            .ok_or_else(|| JsEngineError::new("failed to enumerate V8 object"))?;
        let mut entries = BTreeMap::new();
        for index in 0..names.length() {
            let Some(key_value) = names.get_index(scope, index) else {
                continue;
            };
            let Some(key_string) = key_value.to_string(scope) else {
                continue;
            };
            let key = key_string.to_rust_string_lossy(scope);
            if key == "key" || key == "ref" || key.starts_with("on") {
                continue;
            }
            let Some(item) = object.get(scope, key_value) else {
                continue;
            };
            let item = v8_to_host_inner(scope, item, depth + 1, seen)?;
            if item != HostValue::Undefined {
                entries.insert(key, item);
            }
        }
        seen.remove(&identity);
        return Ok(HostValue::Object(entries));
    }
    Ok(HostValue::Undefined)
}

fn set_object_property(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    key: &str,
    value: &HostValue,
) -> Result<(), JsEngineError> {
    let key = v8::String::new(scope, key)
        .ok_or_else(|| JsEngineError::new("failed to allocate host object key"))?;
    let value = host_to_v8(scope, value)?;
    if object.set(scope, key.into(), value).is_none() {
        return Err(JsEngineError::new("failed to populate host object"));
    }
    Ok(())
}

fn host_resource_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: &HostResourceHandle,
) -> Result<v8::Local<'s, v8::Value>, JsEngineError> {
    let object = v8::Object::new(scope);
    set_object_property(scope, object, "__nanaResource", &HostValue::Bool(true))?;
    let id_key = v8::String::new(scope, "id")
        .ok_or_else(|| JsEngineError::new("failed to allocate resource id key"))?;
    let id = v8::BigInt::new_from_u64(scope, handle.id);
    object.set(scope, id_key.into(), id.into());
    set_object_property(
        scope,
        object,
        "generation",
        &HostValue::Number(handle.generation as f64),
    )?;
    set_object_property(
        scope,
        object,
        "kind",
        &HostValue::String(handle.kind.clone()),
    )?;
    let resources = scope
        .get_slot::<HostApiSlot>()
        .map(|slot| slot.resources.clone())
        .ok_or_else(|| JsEngineError::new("V8 host resource slot is unavailable"))?;
    let released_handle = handle.clone();
    let weak = v8::Weak::with_guaranteed_finalizer(
        scope,
        object,
        Box::new(move || {
            resources.release(&released_handle);
        }),
    );
    scope
        .get_slot::<ResourceFinalizerSlot>()
        .ok_or_else(|| JsEngineError::new("V8 resource finalizer slot is unavailable"))?
        .handles
        .borrow_mut()
        .push(weak);
    Ok(object.into())
}

fn host_resource_from_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<HostResourceHandle> {
    let branded = get_object_property(scope, object, "__nanaResource")?;
    if !branded.boolean_value(scope) {
        return None;
    }
    let id_value = get_object_property(scope, object, "id")?;
    let id = if id_value.is_big_int() {
        let bigint = v8::Local::<v8::BigInt>::try_from(id_value).ok()?;
        let (id, lossless) = bigint.u64_value();
        lossless.then_some(id)?
    } else if id_value.is_number() {
        id_value
            .integer_value(scope)
            .and_then(|id| id.try_into().ok())?
    } else {
        v8_string(scope, id_value).parse().ok()?
    };
    let generation = get_object_property(scope, object, "generation")?.uint32_value(scope)?;
    let kind_value = get_object_property(scope, object, "kind")?;
    let kind = v8_string(scope, kind_value);
    Some(HostResourceHandle::new(id, generation, kind))
}

fn get_object_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = v8::String::new(scope, key)?;
    object.get(scope, key.into())
}

fn normalize_host_args(value: HostValue) -> Vec<HostValue> {
    match value {
        HostValue::Array(values) => values,
        HostValue::Null | HostValue::Undefined => Vec::new(),
        value => vec![value],
    }
}

fn v8_string(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> String {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

fn throw_host_error(scope: &mut v8::PinScope, message: &str) {
    if let Some(message) = v8::String::new(scope, message) {
        let exception = v8::Exception::error(scope, message);
        scope.throw_exception(exception);
    }
}

fn throw_host_exception(scope: &mut v8::PinScope, exception: &JsException) {
    let Some(message) = v8::String::new(scope, &exception.message) else {
        return;
    };
    let error = v8::Exception::error(scope, message);
    if let Ok(object) = v8::Local::<v8::Object>::try_from(error) {
        for (key, value) in [
            ("name", Some(HostValue::string(&exception.name))),
            ("code", exception.code.as_ref().map(HostValue::string)),
            ("details", exception.details.clone()),
        ] {
            let Some(value) = value else { continue };
            let Some(key) = v8::String::new(scope, key) else {
                continue;
            };
            let Ok(value) = host_to_v8(scope, &value) else {
                continue;
            };
            let _ = object.set(scope, key.into(), value);
        }
    }
    scope.throw_exception(error);
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
        return JsEngineError::from_exception(JsException {
            name: "Error".into(),
            code: None,
            message,
            stack,
            details: None,
        });
    }
    if let Some(message) = try_catch.message() {
        return JsEngineError::new(message.get(try_catch).to_rust_string_lossy(try_catch));
    }
    JsEngineError::new("unknown V8 error")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_js_engine::probe::{
        VUE_SFC_COMPAT_CSS, probe_host_registry, vue_runtime_probe_artifact,
        vue_sfc_compat_artifact,
    };
    use std::sync::{Arc, Mutex};

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
    fn reports_unhandled_promise_rejections_with_stack() {
        with_serial_v8_tests(|| {
            let events = Arc::new(Mutex::new(Vec::new()));
            let output = Arc::clone(&events);
            let mut engine = V8Engine::new();
            engine.set_diagnostic_sink(Some(Arc::new(move |event| {
                output.lock().unwrap().push(event);
            })));
            engine
                .initialize(RuntimeArtifact::from_source(
                    "rejection.js",
                    "Promise.reject(new Error('diagnostic rejection'));",
                ))
                .unwrap();
            engine.run_microtasks().unwrap();

            let events = events.lock().unwrap();
            let rejection = events
                .iter()
                .find(|event| event.source == "v8.promise")
                .expect("unhandled rejection diagnostic");
            assert!(rejection.message.contains("diagnostic rejection"));
            assert!(
                rejection
                    .stack
                    .as_deref()
                    .is_some_and(|stack| stack.contains("diagnostic rejection"))
            );
            drop(events);
            engine.shutdown();
        });
    }

    #[test]
    fn inspector_dispatches_chrome_devtools_protocol_messages() {
        with_serial_v8_tests(|| {
            let mut engine = V8Engine::new();
            engine
                .initialize(RuntimeArtifact::from_source(
                    "inspector.js",
                    "globalThis.answer = 42;",
                ))
                .unwrap();
            let transport = engine.enable_inspector().unwrap();
            engine
                .dispatch_inspector_protocol_message(r#"{"id":1,"method":"Runtime.enable"}"#)
                .unwrap();
            let enabled = transport.drain_messages();
            let context_id = enabled
                .iter()
                .filter_map(|message| serde_json::from_str::<serde_json::Value>(message).ok())
                .find_map(|message| {
                    (message.get("method").and_then(serde_json::Value::as_str)
                        == Some("Runtime.executionContextCreated"))
                    .then(|| message.pointer("/params/context/id")?.as_u64())
                    .flatten()
                })
                .expect("inspector execution context");
            engine
                .dispatch_inspector_protocol_message(&format!(
                    r#"{{"id":2,"method":"Runtime.evaluate","params":{{"expression":"answer","contextId":{context_id}}}}}"#
                ))
                .unwrap();
            let messages = transport.drain_messages();
            let response = messages
                .iter()
                .filter_map(|message| serde_json::from_str::<serde_json::Value>(message).ok())
                .find(|message| message.get("id").and_then(serde_json::Value::as_u64) == Some(2))
                .expect("inspector response");
            assert_eq!(
                response
                    .pointer("/result/result/value")
                    .and_then(serde_json::Value::as_u64),
                Some(42),
                "unexpected inspector response: {response}"
            );
            engine.shutdown();
        });
    }

    #[test]
    fn host_bridge_transfers_typed_arrays_without_json() {
        with_serial_v8_tests(|| {
            let mut engine = V8Engine::new();
            let mut api = HostApiRegistry::new();
            api.register("echoBytes", |args| {
                let bytes = args
                    .first()
                    .and_then(HostValue::as_bytes)
                    .ok_or_else(|| JsException::new("expected bytes"))?;
                Ok(HostValue::Bytes(bytes.to_vec()))
            });
            engine.register_host_api(&api).unwrap();
            engine
                .initialize(RuntimeArtifact::from_source(
                    "binary-host.js",
                    r#"
                    globalThis.__binaryHostProbe = {
                      run() {
                        const output = Nana.host.call("echoBytes", [new Uint8Array([0, 7, 255])]);
                        return {
                          isArrayBuffer: output instanceof ArrayBuffer,
                          bytes: Array.from(new Uint8Array(output))
                        };
                      }
                    };
                    "#,
                ))
                .unwrap();
            let run = engine.resolve_function("__binaryHostProbe.run").unwrap();
            let result = engine.invoke(run, &[]).unwrap();
            let object = result.as_object().unwrap();
            assert_eq!(
                object.get("isArrayBuffer").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object.get("bytes"),
                Some(&HostValue::Array(vec![
                    HostValue::Number(0.0),
                    HostValue::Number(7.0),
                    HostValue::Number(255.0),
                ]))
            );
            engine.shutdown();
        });
    }

    #[test]
    fn canvas2d_and_image_resources_execute_inside_v8() {
        with_serial_v8_tests(|| {
            let mut engine = V8Engine::new();
            let mut api = HostApiRegistry::new();
            let state = nana_ui_web_api::shared_web_api_state();
            let canvas = nana_ui_web_api::shared_canvas_runtime();
            nana_ui_web_api::register_web_api_host_ops_with_resources(
                &mut api,
                state,
                nana_ui_web_api::shared_clipboard(nana_ui_web_api::MemoryClipboard::new()),
                canvas,
            );
            engine.register_host_api(&api).unwrap();
            let artifact = nana_ui_web_api::compose_runtime_artifact(
                "canvas-v8.js",
                r##"
                const canvas = document.createElement("canvas");
                canvas.width = 4;
                canvas.height = 4;
                const ctx = canvas.getContext("2d");
                ctx.fillStyle = "#ff0000";
                ctx.fillRect(0, 0, 4, 4);
                ctx.globalCompositeOperation = "destination-out";
                ctx.fillRect(1, 1, 1, 1);
                const pixels = ctx.getImageData(0, 0, 4, 4);
                const url = canvas.toDataURL("image/png");
                globalThis.__canvasProbeState = {
                  canvasType: canvas instanceof HTMLCanvasElement,
                  contextType: ctx instanceof CanvasRenderingContext2D,
                  idType: typeof canvas.__nanaResource.id,
                  first: Array.from(pixels.data.slice(0, 4)),
                  erased: Array.from(pixels.data.slice(20, 24)),
                  dataUrl: url.startsWith("data:image/png;base64,"),
                  bitmap: false,
                  svgLoaded: false,
                  svgError: "",
                };
                createImageBitmap(canvas).then(bitmap => {
                  __canvasProbeState.bitmap = bitmap instanceof ImageBitmap && bitmap.width === 4;
                  bitmap.close();
                });
                const svgImage = new Image();
                svgImage.onload = () => {
                  __canvasProbeState.svgLoaded = svgImage.naturalWidth === 3 && svgImage.naturalHeight === 2;
                  svgImage.close();
                };
                svgImage.onerror = (error) => { __canvasProbeState.svgError = String(error && error.message || error); };
                svgImage.src = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='3' height='2'%3E%3Crect width='3' height='2' fill='%2360a5fa'/%3E%3C/svg%3E";
                globalThis.__canvasProbe = { run: () => __canvasProbeState };
                "##,
            );
            engine.initialize(artifact).unwrap();
            engine.run_microtasks().unwrap();
            let run = engine.resolve_function("__canvasProbe.run").unwrap();
            let result = engine.invoke(run, &[]).unwrap();
            let object = result.as_object().unwrap();
            assert_eq!(
                object.get("canvasType").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object.get("contextType").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object.get("idType").and_then(HostValue::as_str),
                Some("bigint")
            );
            assert_eq!(
                object.get("dataUrl").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object.get("bitmap").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object.get("svgLoaded").and_then(HostValue::as_bool),
                Some(true),
                "SVG Image failed: {:?}",
                object.get("svgError")
            );
            assert_eq!(
                object.get("first"),
                Some(&HostValue::Array(vec![
                    HostValue::Number(255.0),
                    HostValue::Number(0.0),
                    HostValue::Number(0.0),
                    HostValue::Number(255.0),
                ]))
            );
            assert_eq!(
                object.get("erased"),
                Some(&HostValue::Array(vec![
                    HostValue::Number(0.0),
                    HostValue::Number(0.0),
                    HostValue::Number(0.0),
                    HostValue::Number(0.0),
                ]))
            );
            engine.shutdown();
        });
    }

    #[test]
    fn webgpu_facade_reuses_host_device_and_renders_canvas_texture() {
        with_serial_v8_tests(|| {
            use std::sync::Arc;

            use iced::futures::executor;
            use iced::wgpu;
            use nana_ui::HostedGpuResources;

            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::from_env().unwrap_or_default(),
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let adapter =
                executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                    apply_limit_buckets: false,
                }))
                .expect("headless WGPU adapter required for WebGPU behavior test");
            let (device, queue) =
                executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("Nana JS WebGPU test device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::Off,
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                }))
                .expect("headless WGPU device");
            let resources =
                HostedGpuResources::from_existing(adapter, Arc::new(device), Arc::new(queue));

            let mut host = nana_ui_vue::VueHost::new();
            host.bind_host_gpu(resources);
            let api = host.host_api_registry();
            let mut engine = V8Engine::new();
            engine.register_host_api(&api).unwrap();
            let artifact = nana_ui_web_api::compose_runtime_artifact(
                "webgpu-v8.js",
                r#"
                globalThis.__webGpuProbeState = { done: false, error: "" };
                (async () => {
                  const adapter = await navigator.gpu.requestAdapter();
                  const device = await adapter.requestDevice();
                  const upload = device.createBuffer({
                    size: 24,
                    usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.VERTEX,
                  });
                  device.queue.writeBuffer(upload, 0, new Float32Array([
                    -1, -1, 3, -1, -1, 3,
                  ]));
                  let validationError = false;
                   try {
                     device.queue.writeBuffer(upload, 0, new Uint8Array([1, 2, 3]));
                   } catch (error) {
                     validationError = String(error && error.message || error).includes("validation");
                   }
                   const mapped = device.createBuffer({
                     size: 16,
                     usage: GPUBufferUsage.VERTEX,
                     mappedAtCreation: true,
                   });
                   new Uint32Array(mapped.getMappedRange()).set([1, 2, 3, 4]);
                   mapped.unmap();
                   let mapAsyncUnsupported = false;
                   try {
                     await mapped.mapAsync(GPUMapMode.WRITE);
                   } catch (error) {
                     mapAsyncUnsupported = error && error.name === "NotSupportedError";
                   }
                   let asyncPipelineUnsupported = 0;
                   for (const createAsync of [
                     () => device.createRenderPipelineAsync({}),
                     () => device.createComputePipelineAsync({}),
                   ]) {
                     try { await createAsync(); }
                     catch (error) {
                       if (error && error.name === "NotSupportedError") asyncPipelineUnsupported++;
                     }
                   }
                   let invalidFormatRejected = false;
                   try {
                     device.createTexture({
                       size: [1, 1],
                       format: "not-a-format",
                       usage: GPUTextureUsage.TEXTURE_BINDING,
                     });
                   } catch (error) {
                     invalidFormatRejected = String(error && error.message || error).includes("validation");
                   }
                   let malformedDescriptorRejected = 0;
                   for (const create of [
                     () => device.createSampler({ magFilter: 7 }),
                     () => device.createBuffer({ size: 4, usage: GPUBufferUsage.COPY_DST, mappedAtCreation: "yes" }),
                     () => device.createShaderModule({ code: 42 }),
                     () => device.createBindGroupLayout({ entries: {} }),
                   ]) {
                     try { create(); }
                     catch (error) {
                       if (String(error && error.message || error).includes("validation")) malformedDescriptorRejected++;
                     }
                   }
                  const canvas = document.createElement("canvas");
                  canvas.width = 8;
                  canvas.height = 8;
                  const context = canvas.getContext("webgpu");
                  context.configure({
                    device,
                    format: navigator.gpu.getPreferredCanvasFormat(),
                    alphaMode: "premultiplied",
                  });
                  const shader = device.createShaderModule({ code: `
                    @vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> {
                      return vec4(position, 0., 1.);
                    }
                    @fragment fn fs() -> @location(0) vec4<f32> { return vec4(0.2, 0.4, 0.8, 1.0); }
                  ` });
                  const pipeline = device.createRenderPipeline({
                    layout: "auto",
                    vertex: {
                      module: shader,
                      entryPoint: "vs",
                      buffers: [{
                        arrayStride: 8,
                        attributes: [{ shaderLocation: 0, offset: 0, format: "float32x2" }],
                      }],
                    },
                    fragment: {
                      module: shader,
                      entryPoint: "fs",
                      targets: [{
                        format: "rgba8unorm",
                        blend: {
                          color: { srcFactor: "src-alpha", dstFactor: "one-minus-src-alpha", operation: "add" },
                          alpha: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" },
                        },
                      }],
                    },
                    depthStencil: {
                      format: "depth24plus",
                      depthWriteEnabled: true,
                      depthCompare: "less",
                    },
                  });
                  const texture = context.getCurrentTexture();
                  const depth = device.createTexture({
                    size: [8, 8],
                    format: "depth24plus",
                    usage: GPUTextureUsage.RENDER_ATTACHMENT,
                  });
                  const encoder = device.createCommandEncoder();
                  const pass = encoder.beginRenderPass({ colorAttachments: [{
                    view: texture.createView(),
                    loadOp: "clear",
                    clearValue: { r: 0, g: 0, b: 0, a: 0 },
                    storeOp: "store",
                  }], depthStencilAttachment: {
                    view: depth.createView(),
                    depthLoadOp: "clear",
                    depthClearValue: 1,
                    depthStoreOp: "discard",
                  } });
                  pass.setPipeline(pipeline);
                  pass.setViewport(0, 0, 8, 8, 0, 1);
                  pass.setScissorRect(0, 0, 8, 8);
                  pass.setVertexBuffer(0, upload);
                   pass.draw(3);
                   pass.end();
                   device.queue.submit([encoder.finish()]);
                   await device.queue.onSubmittedWorkDone();
                   device.pushErrorScope("validation");
                   device.createShaderModule({ code: "this is not valid WGSL" });
                   const scopedError = await device.popErrorScope();
                   __webGpuProbeState = {
                    done: true,
                    adapter: !!adapter,
                    device: device instanceof GPUDevice,
                    bufferId: typeof upload.id,
                    texture: texture instanceof GPUTexture,
                     slot: texture.__nanaGpuResource.slot,
                     validationError,
                     mapAsyncUnsupported,
                     asyncPipelineUnsupported,
                     invalidFormatRejected,
                     malformedDescriptorRejected,
                     submittedWorkDone: true,
                     scopedValidationError: scopedError instanceof GPUValidationError,
                   };
                })().catch(error => {
                  __webGpuProbeState.error = String(error && (error.stack || error.message) || error);
                });
                globalThis.__webGpuProbe = { run: () => __webGpuProbeState };
                "#,
            );
            engine.initialize(artifact).unwrap();
            for _ in 0..32 {
                host.pump_frame(&mut engine).unwrap();
                engine.run_microtasks().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let run = engine.resolve_function("__webGpuProbe.run").unwrap();
            let result = engine.invoke(run, &[]).unwrap();
            let object = result.as_object().unwrap();
            assert_eq!(
                object.get("error").and_then(HostValue::as_str),
                None,
                "{object:?}"
            );
            assert_eq!(
                object.get("done").and_then(HostValue::as_bool),
                Some(true),
                "{object:?}"
            );
            assert_eq!(
                object.get("device").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object.get("bufferId").and_then(HostValue::as_str),
                Some("bigint")
            );
            assert_eq!(
                object.get("texture").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object.get("validationError").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object
                    .get("mapAsyncUnsupported")
                    .and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object
                    .get("asyncPipelineUnsupported")
                    .and_then(HostValue::as_f64),
                Some(2.0)
            );
            assert_eq!(
                object
                    .get("invalidFormatRejected")
                    .and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object
                    .get("malformedDescriptorRejected")
                    .and_then(HostValue::as_f64),
                Some(4.0)
            );
            assert_eq!(
                object.get("submittedWorkDone").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object
                    .get("scopedValidationError")
                    .and_then(HostValue::as_bool),
                Some(true)
            );
            assert!(
                object
                    .get("slot")
                    .and_then(HostValue::as_str)
                    .is_some_and(|slot| slot.starts_with("webgpu-canvas:"))
            );
            engine.shutdown();
        });
    }

    #[test]
    fn promise_host_calls_and_host_events_settle_on_engine_pump() {
        with_serial_v8_tests(|| {
            let mut engine = V8Engine::new();
            let mut api = HostApiRegistry::new();
            api.register_async("loadBytes", |args, context| {
                let input = args.first().cloned().unwrap_or(HostValue::Null);
                let (completion, pending) = context.pending();
                assert!(completion.resolve(input));
                Ok(pending)
            });
            engine.register_host_api(&api).unwrap();
            engine
                .initialize(RuntimeArtifact::from_source(
                    "async-host.js",
                    r#"
                    globalThis.__asyncHostState = { done: false, bytes: [], events: [] };
                    Nana.host.on("frame", payload => __asyncHostState.events.push(payload.sequence));
                    Nana.host.invoke("loadBytes", [new Uint8Array([4, 2])]).then(value => {
                      __asyncHostState.bytes = Array.from(new Uint8Array(value));
                      __asyncHostState.done = true;
                    });
                    globalThis.__asyncHostProbe = { run: () => __asyncHostState };
                    "#,
                ))
                .unwrap();
            let mut event = BTreeMap::new();
            event.insert("sequence".into(), HostValue::Number(9.0));
            engine
                .host_event_sender()
                .unwrap()
                .send("frame", HostValue::Object(event));
            engine.run_microtasks().unwrap();

            let run = engine.resolve_function("__asyncHostProbe.run").unwrap();
            let result = engine.invoke(run, &[]).unwrap();
            let object = result.as_object().unwrap();
            assert_eq!(object.get("done").and_then(HostValue::as_bool), Some(true));
            assert_eq!(
                object.get("bytes"),
                Some(&HostValue::Array(vec![
                    HostValue::Number(4.0),
                    HostValue::Number(2.0),
                ]))
            );
            assert_eq!(
                object.get("events"),
                Some(&HostValue::Array(vec![HostValue::Number(9.0)]))
            );
            engine.shutdown();
        });
    }

    #[test]
    fn resource_handles_release_and_invalidate_with_context() {
        with_serial_v8_tests(|| {
            let mut engine = V8Engine::new();
            let resources = engine.host_resources().unwrap();
            let handle = resources.insert("image", vec![1_u8, 2, 3]);
            let returned_handle = handle.clone();
            let mut api = HostApiRegistry::new();
            api.register("getResource", move |_| {
                Ok(HostValue::Resource(returned_handle.clone()))
            });
            engine.register_host_api(&api).unwrap();
            engine
                .initialize(RuntimeArtifact::from_source(
                    "resource-host.js",
                    r#"
                    globalThis.__resourceHostProbe = {
                      run() {
                        const handle = Nana.host.call("getResource", []);
                        return {
                          bigintId: typeof handle.id === "bigint",
                          kind: handle.kind,
                          released: Nana.resources.release(handle),
                          releasedTwice: Nana.resources.release(handle)
                        };
                      }
                    };
                    "#,
                ))
                .unwrap();
            let run = engine.resolve_function("__resourceHostProbe.run").unwrap();
            let result = engine.invoke(run, &[]).unwrap();
            let object = result.as_object().unwrap();
            assert_eq!(
                object.get("bigintId").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object.get("kind").and_then(HostValue::as_str),
                Some("image")
            );
            assert_eq!(
                object.get("released").and_then(HostValue::as_bool),
                Some(true)
            );
            assert_eq!(
                object.get("releasedTwice").and_then(HostValue::as_bool),
                Some(false)
            );
            assert!(!resources.contains(&handle));

            let remaining = resources.insert("image", vec![4_u8]);
            assert!(resources.contains(&remaining));
            engine.shutdown();
            assert!(resources.is_empty());
        });
    }

    #[test]
    fn unreachable_resource_objects_release_through_v8_finalizers() {
        with_serial_v8_tests(|| {
            let mut engine = V8Engine::new();
            let resources = engine.host_resources().unwrap();
            let handle = resources.insert("image", vec![7_u8]);
            let returned_handle = handle.clone();
            let mut api = HostApiRegistry::new();
            api.register("getGcResource", move |_| {
                Ok(HostValue::Resource(returned_handle.clone()))
            });
            engine.register_host_api(&api).unwrap();
            engine
                .initialize(RuntimeArtifact::from_source(
                    "resource-finalizer.js",
                    r#"
                    let resource = Nana.host.call("getGcResource", []);
                    globalThis.__dropGcResource = function () { resource = null; };
                    "#,
                ))
                .unwrap();
            assert!(resources.contains(&handle));
            let drop_resource = engine.resolve_function("__dropGcResource").unwrap();
            engine.invoke(drop_resource, &[]).unwrap();
            for _ in 0..4 {
                engine.isolate.as_mut().unwrap().low_memory_notification();
                engine.run_microtasks().unwrap();
                if !resources.contains(&handle) {
                    break;
                }
            }
            assert!(!resources.contains(&handle));
            engine.shutdown();
        });
    }

    #[test]
    fn aborted_promise_cancels_pending_host_work() {
        with_serial_v8_tests(|| {
            let cancellation = Arc::new(Mutex::new(None::<HostCancellationToken>));
            let captured = Arc::clone(&cancellation);
            let mut api = HostApiRegistry::new();
            api.register_async("wait", move |_args, context| {
                *captured.lock().unwrap() = Some(context.cancellation().clone());
                let (_completion, pending) = context.pending();
                Ok(pending)
            });
            let mut engine = V8Engine::new();
            engine.register_host_api(&api).unwrap();
            engine
                .initialize(RuntimeArtifact::from_source(
                    "cancel-host.js",
                    r#"
                    globalThis.__cancelHostState = { error: "" };
                    const signal = {
                      aborted: true,
                      addEventListener() {},
                      removeEventListener() {}
                    };
                    Nana.host.invoke("wait", [], { signal }).catch(error => {
                      __cancelHostState.error = error.name;
                    });
                    globalThis.__cancelHostProbe = { run: () => __cancelHostState };
                    "#,
                ))
                .unwrap();
            engine.run_microtasks().unwrap();
            assert!(
                cancellation
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(HostCancellationToken::is_cancelled)
            );
            let run = engine.resolve_function("__cancelHostProbe.run").unwrap();
            let result = engine.invoke(run, &[]).unwrap();
            assert_eq!(
                result
                    .as_object()
                    .unwrap()
                    .get("error")
                    .and_then(HostValue::as_str),
                Some("AbortError")
            );
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

    #[test]
    fn vue_sfc_fetch_updates_semantic_tree_on_v8() {
        with_serial_v8_tests(|| {
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
                    Ok(FetchResponse {
                        url: request.url,
                        status: 200,
                        status_text: "OK".into(),
                        headers: vec![("content-type".into(), "application/json".into())],
                        body: br#"{"message":"ready","sequence":7}"#.to_vec(),
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
            let mut engine = V8Engine::new();
            let mut application_api = HostApiRegistry::new();
            application_api.register("fixtureApplicationApi", |_| Ok(HostValue::string("v8")));
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
                Some("v8")
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
        });
    }
}
