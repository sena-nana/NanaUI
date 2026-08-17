//! Single-engine, multi-window Vue document coordination.
//!
//! This module deliberately stops at generic window commands. The native host
//! translates them into its existing `HostedWindowCommand` path, so no window,
//! surface, adapter, device, or queue is created by the Vue compatibility layer.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nana_js_engine::{
    HostApiRegistry, HostCallObserver, HostEventSender, HostValue, JsDiagnosticEvent,
    JsDiagnosticLevel, JsDiagnosticSink, JsEngine, JsEngineError, JsException, RuntimeArtifact,
};
use nana_ui_core::ThemeMode;

use crate::{
    CompositionInput, DocumentId, KeyboardInput, NodeHandle, PointerInput, SemanticSnapshot,
    VueHost, WheelInput, WindowLifecycleEvent, compose_vue_artifact,
};
use nana_ui_platform::ImeEvent;

/// Stable JS/native identity for one Vue window. Zero is the primary window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VueWindowId(pub u64);

impl VueWindowId {
    pub const PRIMARY: Self = Self(0);

    fn document_id(self) -> DocumentId {
        DocumentId(self.0.saturating_add(1))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VueWindowRole {
    #[default]
    Main,
    Tool,
    Dialog,
}

/// Engine-neutral native window options accepted from `Nana.windows.create`.
#[derive(Debug, Clone, PartialEq)]
pub struct VueWindowOptions {
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub minimum_width: f64,
    pub minimum_height: f64,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub transparent: bool,
    pub frameless: bool,
    pub always_on_top: bool,
    pub resizable: bool,
    pub modal: bool,
    pub parent: Option<VueWindowId>,
    pub role: VueWindowRole,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VueWindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale_factor: f64,
    pub fullscreen: bool,
    pub minimized: bool,
    pub maximized: bool,
}

impl Default for VueWindowGeometry {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
            fullscreen: false,
            minimized: false,
            maximized: false,
        }
    }
}

impl Default for VueWindowOptions {
    fn default() -> Self {
        Self {
            title: String::new(),
            width: 800.0,
            height: 600.0,
            minimum_width: 320.0,
            minimum_height: 240.0,
            x: None,
            y: None,
            transparent: false,
            frameless: false,
            always_on_top: false,
            resizable: true,
            modal: false,
            parent: None,
            role: VueWindowRole::Main,
        }
    }
}

impl VueWindowOptions {
    fn from_host_value(value: Option<&HostValue>) -> Self {
        let Some(map) = value.and_then(HostValue::as_object) else {
            return Self::default();
        };
        let mut options = Self::default();
        if let Some(title) = map.get("title").and_then(HostValue::as_str) {
            options.title = title.to_string();
        }
        options.width = finite_positive(map.get("width"), options.width);
        options.height = finite_positive(map.get("height"), options.height);
        options.minimum_width = finite_positive(map.get("minimumWidth"), options.minimum_width);
        options.minimum_height = finite_positive(map.get("minimumHeight"), options.minimum_height);
        options.x = finite_number(map.get("x"));
        options.y = finite_number(map.get("y"));
        options.transparent = bool_value(map.get("transparent"), options.transparent);
        options.frameless = bool_value(map.get("frameless"), options.frameless);
        options.always_on_top = bool_value(map.get("alwaysOnTop"), options.always_on_top);
        options.resizable = bool_value(map.get("resizable"), options.resizable);
        options.modal = bool_value(map.get("modal"), options.modal);
        options.parent = map
            .get("parentId")
            .and_then(HostValue::as_f64)
            .filter(|id| id.is_finite() && *id >= 0.0)
            .map(|id| VueWindowId(id as u64));
        options.role = match map.get("role").and_then(HostValue::as_str) {
            Some("tool") => VueWindowRole::Tool,
            Some("dialog") => VueWindowRole::Dialog,
            _ => VueWindowRole::Main,
        };
        options
    }

    fn to_host_value(&self, id: VueWindowId, mount_root: NodeHandle) -> HostValue {
        HostValue::Object(
            [
                ("id".into(), HostValue::Number(id.0 as f64)),
                ("mountRoot".into(), HostValue::Number(mount_root.0 as f64)),
                ("width".into(), HostValue::Number(self.width)),
                ("height".into(), HostValue::Number(self.height)),
                ("ready".into(), HostValue::Bool(false)),
            ]
            .into_iter()
            .collect(),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VueWindowCommand {
    Open {
        id: VueWindowId,
        options: VueWindowOptions,
    },
    Close(VueWindowId),
    Focus(VueWindowId),
    Move {
        id: VueWindowId,
        x: f64,
        y: f64,
    },
    SetTitle {
        id: VueWindowId,
        title: String,
    },
    SetBounds {
        id: VueWindowId,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    SetFullscreen {
        id: VueWindowId,
        fullscreen: bool,
    },
    SetMinimized {
        id: VueWindowId,
        minimized: bool,
    },
    SetMaximized {
        id: VueWindowId,
        maximized: bool,
    },
    SetAlwaysOnTop {
        id: VueWindowId,
        always_on_top: bool,
    },
}

struct WindowEntry {
    host: Arc<Mutex<VueHost>>,
    api: HostApiRegistry,
    options: VueWindowOptions,
    geometry: VueWindowGeometry,
    ready: bool,
}

struct VueRuntimeState {
    windows: BTreeMap<VueWindowId, WindowEntry>,
    canvas: nana_ui_web_api::SharedCanvasRuntime,
    local_storage: nana_ui_web_api::SharedStorage,
    stylesheets: Vec<String>,
    #[cfg(feature = "iced-view")]
    components: crate::NativeComponentRegistry,
    #[cfg(feature = "iced-view")]
    host_textures: nana_ui::HostTextureRegistry,
    #[cfg(feature = "hosted")]
    webgpu: Option<crate::JsWebGpuRuntime>,
    #[cfg(feature = "hosted")]
    canvas_gpu: Option<crate::canvas_gpu::CanvasGpuBridge>,
    next_id: u64,
    commands: VecDeque<VueWindowCommand>,
    events: Option<HostEventSender>,
    diagnostic_sink: Option<JsDiagnosticSink>,
    host_call_observer: Option<HostCallObserver>,
}

impl VueRuntimeState {
    fn create_window(
        &mut self,
        options: VueWindowOptions,
    ) -> Result<(VueWindowId, NodeHandle), JsException> {
        // 2^21 document namespaces * 2^32 local ids stay below Number::MAX_SAFE_INTEGER.
        if self.next_id >= (1 << 21) - 1 {
            return Err(JsException::new("Vue window id space exhausted"));
        }
        if let Some(parent) = options.parent
            && !self.windows.contains_key(&parent)
        {
            return Err(JsException::new("parent Vue window does not exist"));
        }
        let id = VueWindowId(self.next_id);
        self.next_id += 1;
        let mut host = VueHost::with_document_id_and_shared_resources(
            id.document_id(),
            options.width.round().max(1.0) as u32,
            options.height.round().max(1.0) as u32,
            1.0,
            Arc::clone(&self.canvas),
            Arc::clone(&self.local_storage),
        );
        #[cfg(feature = "iced-view")]
        {
            host.share_components(self.components.clone());
            host.share_host_textures(self.host_textures.clone());
        }
        #[cfg(feature = "hosted")]
        {
            if let Some(webgpu) = &self.webgpu {
                host.share_webgpu_runtime(webgpu.clone());
            }
            if let Some(canvas_gpu) = &self.canvas_gpu {
                host.share_canvas_gpu(canvas_gpu.clone());
            }
        }
        host.set_diagnostics(
            self.diagnostic_sink.clone(),
            self.host_call_observer.clone(),
        );
        for stylesheet in &self.stylesheets {
            host.inject_stylesheet(stylesheet);
        }
        let mount_root = host.mount_root();
        let api = host.host_api_registry();
        self.windows.insert(
            id,
            WindowEntry {
                host: Arc::new(Mutex::new(host)),
                api,
                geometry: VueWindowGeometry {
                    width: options.width,
                    height: options.height,
                    ..VueWindowGeometry::default()
                },
                options: options.clone(),
                ready: false,
            },
        );
        self.commands
            .push_back(VueWindowCommand::Open { id, options });
        if let Some(sink) = &self.diagnostic_sink {
            sink(JsDiagnosticEvent {
                source: "nana.window".into(),
                level: JsDiagnosticLevel::Info,
                message: format!("window created: {}", id.0),
                stack: None,
            });
        }
        Ok((id, mount_root))
    }

    fn host(&self, id: VueWindowId) -> Result<Arc<Mutex<VueHost>>, JsException> {
        self.windows
            .get(&id)
            .map(|entry| Arc::clone(&entry.host))
            .ok_or_else(|| JsException::new(format!("unknown Vue window {}", id.0)))
    }

    #[cfg(feature = "hosted")]
    fn emit(&self, name: &str, payload: HostValue) {
        if let Some(events) = &self.events {
            events.send(name, payload);
        }
    }

    fn emit_reliable(&self, name: &str, payload: HostValue) -> Result<(), JsEngineError> {
        if let Some(events) = &self.events {
            events
                .send_reliable(name, payload)
                .map_err(|_| JsEngineError::new(format!("host event queue is full: {name}")))?;
        }
        Ok(())
    }
}

/// Owns all Vue window roots that execute inside one attached JS engine.
pub struct VueRuntime {
    state: Arc<Mutex<VueRuntimeState>>,
}

impl Default for VueRuntime {
    fn default() -> Self {
        Self::new(800, 600, 1.0)
    }
}

impl VueRuntime {
    pub fn new(physical_width: u32, physical_height: u32, scale_factor: f32) -> Self {
        let canvas = nana_ui_web_api::shared_canvas_runtime();
        let local_storage = nana_ui_web_api::shared_storage();
        let primary = VueHost::with_document_id_and_shared_resources(
            VueWindowId::PRIMARY.document_id(),
            physical_width,
            physical_height,
            scale_factor,
            Arc::clone(&canvas),
            Arc::clone(&local_storage),
        );
        #[cfg(feature = "iced-view")]
        let mut primary = primary;
        #[cfg(feature = "iced-view")]
        let components = crate::NativeComponentRegistry::new();
        #[cfg(feature = "iced-view")]
        let host_textures = nana_ui::HostTextureRegistry::new();
        #[cfg(feature = "iced-view")]
        {
            primary.share_components(components.clone());
            primary.share_host_textures(host_textures.clone());
        }
        let primary_api = primary.host_api_registry();
        Self {
            state: Arc::new(Mutex::new(VueRuntimeState {
                windows: [(
                    VueWindowId::PRIMARY,
                    WindowEntry {
                        host: Arc::new(Mutex::new(primary)),
                        api: primary_api,
                        options: VueWindowOptions {
                            width: physical_width as f64 / f64::from(scale_factor.max(0.01)),
                            height: physical_height as f64 / f64::from(scale_factor.max(0.01)),
                            ..VueWindowOptions::default()
                        },
                        geometry: VueWindowGeometry {
                            width: physical_width as f64 / f64::from(scale_factor.max(0.01)),
                            height: physical_height as f64 / f64::from(scale_factor.max(0.01)),
                            scale_factor: f64::from(scale_factor.max(0.01)),
                            ..VueWindowGeometry::default()
                        },
                        ready: true,
                    },
                )]
                .into_iter()
                .collect(),
                canvas,
                local_storage,
                stylesheets: Vec::new(),
                #[cfg(feature = "iced-view")]
                components,
                #[cfg(feature = "iced-view")]
                host_textures,
                #[cfg(feature = "hosted")]
                webgpu: None,
                #[cfg(feature = "hosted")]
                canvas_gpu: None,
                next_id: 1,
                commands: VecDeque::new(),
                events: None,
                diagnostic_sink: None,
                host_call_observer: None,
            })),
        }
    }

    pub fn window_ids(&self) -> Vec<VueWindowId> {
        self.state
            .lock()
            .expect("Vue runtime state")
            .windows
            .keys()
            .copied()
            .collect()
    }

    #[cfg(feature = "iced-view")]
    pub fn components(&self) -> crate::NativeComponentRegistry {
        self.state
            .lock()
            .expect("Vue runtime state")
            .components
            .clone()
    }

    /// Move view-time Rust/Iced failures onto the owning V8 event queue. The
    /// JS runtime turns these into component-local `error` events and a global
    /// `Nana.components.onError` notification.
    #[cfg(feature = "iced-view")]
    pub fn flush_native_component_failures(&self) -> Result<usize, JsEngineError> {
        let failures = self.components().drain_failures();
        let count = failures.len();
        let Ok(state) = self.state.lock() else {
            return Err(JsEngineError::new("Vue runtime state poisoned"));
        };
        for (index, failure) in failures.iter().enumerate() {
            let document = crate::DocumentId::from_node(crate::NodeHandle(failure.id));
            let window_id = document.0.saturating_sub(1);
            if let Err(error) = state.emit_reliable(
                "native-component-error",
                HostValue::Object(
                    [
                        ("windowId".into(), HostValue::Number(window_id as f64)),
                        ("id".into(), HostValue::Number(failure.id as f64)),
                        (
                            "component".into(),
                            HostValue::String(failure.component.clone()),
                        ),
                        ("error".into(), failure.error.to_host_value()),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ) {
                drop(state);
                self.components()
                    .restore_failures(failures[index..].to_vec());
                return Err(error);
            }
        }
        Ok(count)
    }

    #[cfg(feature = "iced-view")]
    pub fn has_native_component_failures(&self) -> bool {
        self.components().has_failures()
    }

    #[cfg(feature = "hosted")]
    pub fn bind_host_gpu(
        &self,
        resources: nana_ui::HostedGpuResources,
    ) -> Result<u64, JsEngineError> {
        let hosts = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?
            .windows
            .values()
            .map(|entry| Arc::clone(&entry.host))
            .collect::<Vec<_>>();
        let primary = hosts
            .first()
            .ok_or_else(|| JsEngineError::new("primary Vue window is missing"))?;
        let runtime = {
            let mut primary = primary
                .lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?;
            primary.bind_host_gpu(resources);
            primary
                .webgpu_runtime()
                .cloned()
                .ok_or_else(|| JsEngineError::new("failed to bind shared WebGPU runtime"))?
        };
        let canvas_gpu = primary
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .canvas_gpu()
            .cloned()
            .ok_or_else(|| JsEngineError::new("failed to bind shared Canvas GPU runtime"))?;
        for host in hosts.into_iter().skip(1) {
            let mut host = host
                .lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?;
            host.share_webgpu_runtime(runtime.clone());
            host.share_canvas_gpu(canvas_gpu.clone());
        }
        if let Ok(mut state) = self.state.lock() {
            state.webgpu = Some(runtime.clone());
            state.canvas_gpu = Some(canvas_gpu);
            for entry in state.windows.values_mut() {
                if let Ok(host) = entry.host.lock() {
                    entry.api = host.host_api_registry();
                }
            }
        }
        Ok(runtime.generation())
    }

    #[cfg(feature = "hosted")]
    pub fn replace_host_gpu<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        resources: nana_ui::HostedGpuResources,
        message: &str,
    ) -> Result<u64, JsEngineError> {
        let hosts = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?
            .windows
            .values()
            .map(|entry| Arc::clone(&entry.host))
            .collect::<Vec<_>>();
        let primary = hosts
            .first()
            .ok_or_else(|| JsEngineError::new("primary Vue window is missing"))?;
        let generation = {
            let mut primary = primary
                .lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?;
            primary.replace_host_gpu(engine, resources, message)?
        };
        let runtime = primary
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .webgpu_runtime()
            .cloned()
            .ok_or_else(|| JsEngineError::new("failed to replace shared WebGPU runtime"))?;
        let canvas_gpu = primary
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .canvas_gpu()
            .cloned()
            .ok_or_else(|| JsEngineError::new("failed to replace shared Canvas GPU runtime"))?;
        for host in hosts.into_iter().skip(1) {
            let mut host = host
                .lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?;
            host.share_webgpu_runtime(runtime.clone());
            host.share_canvas_gpu(canvas_gpu.clone());
        }
        if let Ok(mut state) = self.state.lock() {
            state.webgpu = Some(runtime);
            state.canvas_gpu = Some(canvas_gpu);
            for entry in state.windows.values_mut() {
                if let Ok(host) = entry.host.lock() {
                    entry.api = host.host_api_registry();
                }
            }
        }
        Ok(generation)
    }

    pub fn host(&self, id: VueWindowId) -> Option<Arc<Mutex<VueHost>>> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.windows.get(&id).map(|entry| Arc::clone(&entry.host)))
    }

    pub fn shared_runtime_document(
        &self,
        id: VueWindowId,
    ) -> Option<Arc<crate::SharedRuntimeDocument>> {
        self.host(id)
            .and_then(|host| host.lock().ok().map(|host| host.shared_runtime_document()))
    }

    /// Registers application CSS for every current and future Vue window.
    pub fn inject_stylesheet(&self, css: &str) -> Result<(), JsEngineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?;
        for entry in state.windows.values() {
            entry
                .host
                .lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                .inject_stylesheet(css);
        }
        state.stylesheets.push(css.to_owned());
        Ok(())
    }

    /// Apply diagnostics to every existing window and inherit them for windows
    /// created later inside the same V8 context.
    pub fn set_diagnostics(
        &self,
        sink: Option<JsDiagnosticSink>,
        host_calls: Option<HostCallObserver>,
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.diagnostic_sink = sink.clone();
        state.host_call_observer = host_calls.clone();
        for entry in state.windows.values_mut() {
            if let Ok(mut host) = entry.host.lock() {
                host.set_diagnostics(sink.clone(), host_calls.clone());
                entry.api = host.host_api_registry();
            }
        }
    }

    pub fn semantic_snapshot(&self, id: VueWindowId) -> Option<SemanticSnapshot> {
        let host = self.host(id)?;
        let snapshot = host.lock().ok()?.semantic_snapshot();
        Some(snapshot)
    }

    /// Framework registry: primary-document DOM ops plus explicit multi-window routing.
    pub fn host_api_registry(&self) -> HostApiRegistry {
        let mut api = self
            .state
            .lock()
            .expect("Vue runtime state")
            .windows
            .get(&VueWindowId::PRIMARY)
            .expect("primary Vue window")
            .api
            .clone();

        {
            let state = Arc::clone(&self.state);
            api.register("windowCall", move |args| {
                let id = window_id_arg(args.first())?;
                let operation = args
                    .get(1)
                    .and_then(HostValue::as_str)
                    .ok_or_else(|| JsException::new("missing Vue window operation"))?;
                let call_args = args
                    .get(2)
                    .and_then(HostValue::as_array)
                    .cloned()
                    .unwrap_or_default();
                let registry = state
                    .lock()
                    .map_err(state_poisoned)?
                    .windows
                    .get(&id)
                    .map(|entry| entry.api.clone())
                    .ok_or_else(|| JsException::new(format!("unknown Vue window {}", id.0)))?;
                registry.call(operation, &call_args)
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowCreate", move |args| {
                let options = VueWindowOptions::from_host_value(args.first());
                let (id, mount_root) = state
                    .lock()
                    .map_err(state_poisoned)?
                    .create_window(options.clone())?;
                Ok(options.to_host_value(id, mount_root))
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowClose", move |args| {
                let id = window_id_arg(args.first())?;
                let mut state = state.lock().map_err(state_poisoned)?;
                state.host(id)?;
                state.commands.push_back(VueWindowCommand::Close(id));
                Ok(HostValue::Null)
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowFocus", move |args| {
                let id = window_id_arg(args.first())?;
                let mut state = state.lock().map_err(state_poisoned)?;
                state.host(id)?;
                state.commands.push_back(VueWindowCommand::Focus(id));
                Ok(HostValue::Null)
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowMove", move |args| {
                let id = window_id_arg(args.first())?;
                let x = finite_number(args.get(1)).unwrap_or(0.0);
                let y = finite_number(args.get(2)).unwrap_or(0.0);
                let mut state = state.lock().map_err(state_poisoned)?;
                state.host(id)?;
                state
                    .commands
                    .push_back(VueWindowCommand::Move { id, x, y });
                Ok(HostValue::Null)
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowSetTitle", move |args| {
                let id = window_id_arg(args.first())?;
                let title = args
                    .get(1)
                    .and_then(HostValue::as_str)
                    .unwrap_or_default()
                    .to_string();
                let mut state = state.lock().map_err(state_poisoned)?;
                let entry = state
                    .windows
                    .get_mut(&id)
                    .ok_or_else(|| JsException::new(format!("unknown Vue window {}", id.0)))?;
                entry.options.title.clone_from(&title);
                state
                    .commands
                    .push_back(VueWindowCommand::SetTitle { id, title });
                Ok(HostValue::Null)
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowSetBounds", move |args| {
                let id = window_id_arg(args.first())?;
                let x = finite_number(args.get(1)).unwrap_or(0.0);
                let y = finite_number(args.get(2)).unwrap_or(0.0);
                let width = finite_positive(args.get(3), 1.0);
                let height = finite_positive(args.get(4), 1.0);
                let mut state = state.lock().map_err(state_poisoned)?;
                let entry = state
                    .windows
                    .get_mut(&id)
                    .ok_or_else(|| JsException::new(format!("unknown Vue window {}", id.0)))?;
                entry.geometry.x = x;
                entry.geometry.y = y;
                entry.geometry.width = width;
                entry.geometry.height = height;
                state.commands.push_back(VueWindowCommand::SetBounds {
                    id,
                    x,
                    y,
                    width,
                    height,
                });
                Ok(HostValue::Null)
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowSetFullscreen", move |args| {
                push_flag_command(&state, args, |id, fullscreen| {
                    VueWindowCommand::SetFullscreen { id, fullscreen }
                })
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowSetMinimized", move |args| {
                push_flag_command(&state, args, |id, minimized| {
                    VueWindowCommand::SetMinimized { id, minimized }
                })
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowSetMaximized", move |args| {
                push_flag_command(&state, args, |id, maximized| {
                    VueWindowCommand::SetMaximized { id, maximized }
                })
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowSetAlwaysOnTop", move |args| {
                push_flag_command(&state, args, |id, always_on_top| {
                    VueWindowCommand::SetAlwaysOnTop { id, always_on_top }
                })
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowGeometry", move |args| {
                let id = window_id_arg(args.first())?;
                let state = state.lock().map_err(state_poisoned)?;
                let entry = state
                    .windows
                    .get(&id)
                    .ok_or_else(|| JsException::new(format!("unknown Vue window {}", id.0)))?;
                Ok(geometry_value(&entry.geometry))
            });
        }
        {
            let state = Arc::clone(&self.state);
            api.register("windowList", move |_| {
                let state = state.lock().map_err(state_poisoned)?;
                Ok(HostValue::Array(
                    state
                        .windows
                        .iter()
                        .map(|(id, entry)| {
                            HostValue::Object(
                                [
                                    ("id".into(), HostValue::Number(id.0 as f64)),
                                    ("ready".into(), HostValue::Bool(entry.ready)),
                                    (
                                        "mountRoot".into(),
                                        HostValue::Number(
                                            entry.host.lock().expect("Vue host").mount_root().0
                                                as f64,
                                        ),
                                    ),
                                ]
                                .into_iter()
                                .collect(),
                            )
                        })
                        .collect(),
                ))
            });
        }
        api
    }

    pub fn attach_engine<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        engine.register_host_api(&self.host_api_registry())?;
        if let Ok(mut state) = self.state.lock() {
            state.events = engine.host_event_sender();
        }
        self.bind_event_bridges(engine)
    }

    pub fn initialize<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        artifact: RuntimeArtifact,
        application_api: &HostApiRegistry,
    ) -> Result<(), JsEngineError> {
        let mut api = self.host_api_registry();
        api.try_extend(application_api)?;
        engine.register_host_api(&api)?;
        if let Ok(mut state) = self.state.lock() {
            state.events = engine.host_event_sender();
        }
        if artifact.is_binary_release() {
            engine.initialize(artifact)?;
        } else {
            let source = artifact.source_utf8()?;
            let composed = if source.contains("__nanaWebApi") {
                artifact
            } else {
                compose_vue_artifact(artifact.name.clone(), source)
            };
            engine.initialize(composed)?;
        }
        self.bind_event_bridges(engine)
    }

    fn bind_event_bridges<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        let hosts = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?
            .windows
            .iter()
            .map(|(id, entry)| (*id, Arc::clone(&entry.host)))
            .collect::<Vec<_>>();
        for (id, host) in hosts {
            let mut host = host
                .lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?;
            if id == VueWindowId::PRIMARY {
                host.bind_event_bridge(engine)?;
            } else {
                host.bind_event_bridge_for_window(engine, id.0)?;
            }
        }
        Ok(())
    }

    /// Bind JS callbacks for windows created after engine initialization.
    pub fn bind_window<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        id: VueWindowId,
    ) -> Result<(), JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        if id == VueWindowId::PRIMARY {
            host.lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                .bind_event_bridge(engine)
        } else {
            host.lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                .bind_event_bridge_for_window(engine, id.0)
        }
    }

    pub fn drain_window_commands(&self) -> Vec<VueWindowCommand> {
        let Ok(mut state) = self.state.lock() else {
            return Vec::new();
        };
        state.commands.drain(..).collect()
    }

    /// Translate Vue window requests into the Scene/`run_runtime` host contract.
    pub fn drain_runtime_window_commands(&self) -> Vec<nana_ui_platform::WindowCommand> {
        use nana_ui_platform::{WindowCommand, WindowId, WindowRole, WindowSettings};

        self.drain_window_commands()
            .into_iter()
            .map(|command| match command {
                VueWindowCommand::Open { id, options } => WindowCommand::Open {
                    id: WindowId(id.0),
                    settings: WindowSettings {
                        title: options.title,
                        initial_size: (options.width, options.height),
                        minimum_size: (options.minimum_width, options.minimum_height),
                        initial_position: match (options.x, options.y) {
                            (Some(x), Some(y)) => Some((x, y)),
                            _ => None,
                        },
                        maximized: false,
                        transparent: options.transparent,
                        always_on_top: options.always_on_top,
                        resizable: options.resizable,
                        role: match options.role {
                            VueWindowRole::Main => WindowRole::Main,
                            VueWindowRole::Tool | VueWindowRole::Dialog => WindowRole::Tool,
                        },
                        modal: options.modal,
                        parent: options.parent.map(|parent| WindowId(parent.0)),
                    },
                },
                VueWindowCommand::Close(id) => WindowCommand::Close(WindowId(id.0)),
                VueWindowCommand::Focus(id) => WindowCommand::Focus(WindowId(id.0)),
                VueWindowCommand::Move { id, x, y } => WindowCommand::Move {
                    id: WindowId(id.0),
                    position: (x as f32, y as f32),
                },
                VueWindowCommand::SetTitle { id, title } => WindowCommand::SetTitle {
                    id: WindowId(id.0),
                    title,
                },
                VueWindowCommand::SetBounds {
                    id,
                    x,
                    y,
                    width,
                    height,
                } => WindowCommand::SetBounds {
                    id: WindowId(id.0),
                    position: (x as f32, y as f32),
                    size: (width as f32, height as f32),
                },
                VueWindowCommand::SetFullscreen { id, fullscreen } => {
                    WindowCommand::SetFullscreen {
                        id: WindowId(id.0),
                        fullscreen,
                    }
                }
                VueWindowCommand::SetMinimized { id, minimized } => WindowCommand::SetMinimized {
                    id: WindowId(id.0),
                    minimized,
                },
                VueWindowCommand::SetMaximized { id, maximized } => WindowCommand::SetMaximized {
                    id: WindowId(id.0),
                    maximized,
                },
                VueWindowCommand::SetAlwaysOnTop { id, always_on_top } => {
                    WindowCommand::SetAlwaysOnTop {
                        id: WindowId(id.0),
                        always_on_top,
                    }
                }
            })
            .collect()
    }

    pub fn notify_window_ready(&self, id: VueWindowId) -> Result<(), JsEngineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?;
        let entry = state
            .windows
            .get_mut(&id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        entry.ready = true;
        state.emit_reliable(
            "window-ready",
            HostValue::Object(
                [("id".into(), HostValue::Number(id.0 as f64))]
                    .into_iter()
                    .collect(),
            ),
        )?;
        Ok(())
    }

    /// Releases the document only after the native host confirms close.
    pub fn notify_window_closed(&self, id: VueWindowId) -> Result<(), JsEngineError> {
        if id == VueWindowId::PRIMARY {
            return Err(JsEngineError::new(
                "primary Vue window is released with the runtime",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?;
        if state.windows.remove(&id).is_none() {
            return Err(JsEngineError::new(format!("unknown Vue window {}", id.0)));
        }
        if let Some(sink) = &state.diagnostic_sink {
            sink(JsDiagnosticEvent {
                source: "nana.window".into(),
                level: JsDiagnosticLevel::Info,
                message: format!("window released: {}", id.0),
                stack: None,
            });
        }
        state.emit_reliable(
            "window-closed",
            HostValue::Object(
                [("id".into(), HostValue::Number(id.0 as f64))]
                    .into_iter()
                    .collect(),
            ),
        )?;
        Ok(())
    }

    /// Reject a provisional JS window after native creation fails and release
    /// its document/component state immediately.
    pub fn notify_window_open_failed(
        &self,
        id: VueWindowId,
        message: impl Into<String>,
    ) -> Result<(), JsEngineError> {
        if id == VueWindowId::PRIMARY {
            return Err(JsEngineError::new(
                "primary Vue window cannot fail after runtime creation",
            ));
        }
        let message = message.into();
        let mut state = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?;
        if state.windows.remove(&id).is_none() {
            return Err(JsEngineError::new(format!("unknown Vue window {}", id.0)));
        }
        state.emit_reliable(
            "window-open-failed",
            HostValue::Object(
                [
                    ("id".into(), HostValue::Number(id.0 as f64)),
                    ("message".into(), HostValue::String(message)),
                ]
                .into_iter()
                .collect(),
            ),
        )?;
        Ok(())
    }

    pub fn request_close(&self, id: VueWindowId) -> Result<(), JsEngineError> {
        if id == VueWindowId::PRIMARY {
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?;
        state
            .host(id)
            .map_err(|error| JsEngineError::new(error.to_string()))?;
        state.commands.push_back(VueWindowCommand::Close(id));
        Ok(())
    }

    #[cfg(feature = "hosted")]
    pub fn record_geometry(
        &self,
        id: VueWindowId,
        geometry: &nana_ui_platform::WindowGeometry,
    ) -> Result<(), JsEngineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?;
        let entry = state
            .windows
            .get_mut(&id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        entry.geometry.width = geometry.logical_size.0 as f64;
        entry.geometry.height = geometry.logical_size.1 as f64;
        entry.geometry.scale_factor = geometry.scale_factor as f64;
        entry.geometry.maximized = geometry.maximized;
        if let Some((x, y)) = geometry.logical_position {
            entry.geometry.x = x as f64;
            entry.geometry.y = y as f64;
        }
        let payload = geometry_value(&entry.geometry);
        if let HostValue::Object(mut map) = payload {
            map.insert("id".into(), HostValue::Number(id.0 as f64));
            state.emit("window-geometry", HostValue::Object(map));
        }
        Ok(())
    }

    #[cfg(feature = "hosted")]
    pub fn record_platform_geometry(
        &self,
        id: VueWindowId,
        geometry: &nana_ui_platform::WindowGeometry,
    ) -> Result<(), JsEngineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| JsEngineError::new("Vue runtime state poisoned"))?;
        let entry = state
            .windows
            .get_mut(&id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        entry.geometry.width = geometry.logical_size.0 as f64;
        entry.geometry.height = geometry.logical_size.1 as f64;
        entry.geometry.scale_factor = geometry.scale_factor as f64;
        entry.geometry.maximized = geometry.maximized;
        if let Some((x, y)) = geometry.logical_position {
            entry.geometry.x = x as f64;
            entry.geometry.y = y as f64;
        }
        let payload = geometry_value(&entry.geometry);
        if let HostValue::Object(mut map) = payload {
            map.insert("id".into(), HostValue::Number(id.0 as f64));
            state.emit("window-geometry", HostValue::Object(map));
        }
        Ok(())
    }

    pub fn set_viewport(
        &self,
        id: VueWindowId,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
    ) -> Result<(), JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        host.lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .set_viewport(physical_width, physical_height, scale_factor);
        Ok(())
    }

    pub fn pump_lifecycle<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        id: VueWindowId,
        event: WindowLifecycleEvent,
    ) -> Result<bool, JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        let result = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .pump_lifecycle(engine, event)?;
        Ok(result)
    }

    pub fn dispatch_pointer<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        id: VueWindowId,
        input: PointerInput,
    ) -> Result<bool, JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        let result = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .dispatch_pointer(engine, input)?;
        Ok(result)
    }

    pub fn dispatch_wheel<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        id: VueWindowId,
        input: WheelInput,
    ) -> Result<bool, JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        let result = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .dispatch_wheel(engine, input)?;
        Ok(result)
    }

    pub fn dispatch_keyboard<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        id: VueWindowId,
        input: &KeyboardInput,
        target: Option<NodeHandle>,
    ) -> Result<bool, JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        let result = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .dispatch_keyboard(engine, input, target)?;
        Ok(result)
    }

    pub fn dispatch_composition<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        id: VueWindowId,
        input: &CompositionInput,
    ) -> Result<bool, JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        let result = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .dispatch_composition(engine, input)?;
        Ok(result)
    }

    pub fn dispatch_native_ime<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        id: VueWindowId,
        event: &ImeEvent,
    ) -> Result<bool, JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        let result = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .dispatch_native_ime(engine, event)?;
        Ok(result)
    }

    pub fn inject_theme<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        id: VueWindowId,
        theme: ThemeMode,
    ) -> Result<(), JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        let result = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .inject_theme(engine, theme);
        result
    }

    pub fn pump_frame<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
        id: VueWindowId,
    ) -> Result<usize, JsEngineError> {
        let host = self
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        let result = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .pump_frame(engine)?;
        Ok(result)
    }

    pub fn next_wakeup(&self) -> Option<Instant> {
        let hosts = self
            .state
            .lock()
            .ok()?
            .windows
            .values()
            .map(|entry| Arc::clone(&entry.host))
            .collect::<Vec<_>>();
        hosts
            .into_iter()
            .filter_map(|host| host.lock().ok()?.next_wakeup())
            .min()
    }
}

fn window_id_arg(value: Option<&HostValue>) -> Result<VueWindowId, JsException> {
    value
        .and_then(HostValue::as_f64)
        .filter(|id| id.is_finite() && *id >= 0.0 && id.fract() == 0.0)
        .map(|id| VueWindowId(id as u64))
        .ok_or_else(|| JsException::new("missing or invalid Vue window id"))
}

fn finite_number(value: Option<&HostValue>) -> Option<f64> {
    value
        .and_then(HostValue::as_f64)
        .filter(|value| value.is_finite())
}

fn finite_positive(value: Option<&HostValue>, fallback: f64) -> f64 {
    finite_number(value)
        .filter(|value| *value > 0.0)
        .unwrap_or(fallback)
}

fn bool_value(value: Option<&HostValue>, fallback: bool) -> bool {
    value.and_then(HostValue::as_bool).unwrap_or(fallback)
}

fn geometry_value(geometry: &VueWindowGeometry) -> HostValue {
    HostValue::Object(
        [
            ("x".into(), HostValue::Number(geometry.x)),
            ("y".into(), HostValue::Number(geometry.y)),
            ("width".into(), HostValue::Number(geometry.width)),
            ("height".into(), HostValue::Number(geometry.height)),
            (
                "scaleFactor".into(),
                HostValue::Number(geometry.scale_factor),
            ),
            ("fullscreen".into(), HostValue::Bool(geometry.fullscreen)),
            ("minimized".into(), HostValue::Bool(geometry.minimized)),
            ("maximized".into(), HostValue::Bool(geometry.maximized)),
        ]
        .into_iter()
        .collect(),
    )
}

fn push_flag_command(
    state: &Arc<Mutex<VueRuntimeState>>,
    args: &[HostValue],
    command: impl FnOnce(VueWindowId, bool) -> VueWindowCommand,
) -> Result<HostValue, JsException> {
    let id = window_id_arg(args.first())?;
    let flag = bool_value(args.get(1), true);
    let mut state = state.lock().map_err(state_poisoned)?;
    let entry = state
        .windows
        .get_mut(&id)
        .ok_or_else(|| JsException::new(format!("unknown Vue window {}", id.0)))?;
    let command = command(id, flag);
    match &command {
        VueWindowCommand::SetFullscreen { fullscreen, .. } => {
            entry.geometry.fullscreen = *fullscreen;
        }
        VueWindowCommand::SetMinimized { minimized, .. } => {
            entry.geometry.minimized = *minimized;
        }
        VueWindowCommand::SetMaximized { maximized, .. } => {
            entry.geometry.maximized = *maximized;
        }
        VueWindowCommand::SetAlwaysOnTop { always_on_top, .. } => {
            entry.options.always_on_top = *always_on_top;
        }
        _ => {}
    }
    state.commands.push_back(command);
    Ok(HostValue::Null)
}

fn state_poisoned<T>(_error: std::sync::PoisonError<T>) -> JsException {
    JsException::new("Vue runtime state poisoned")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NODE_HANDLE_DOCUMENT_STRIDE;

    #[test]
    fn registry_creates_independent_routable_window_documents() {
        let runtime = VueRuntime::new(1200, 800, 1.0);
        runtime
            .inject_stylesheet("section { color: rgb(255, 0, 0); }")
            .unwrap();
        let api = runtime.host_api_registry();
        let created = api
            .call(
                "windowCreate",
                &[HostValue::Object(
                    [
                        ("title".into(), HostValue::string("Tool")),
                        ("width".into(), HostValue::Number(420.0)),
                        ("height".into(), HostValue::Number(300.0)),
                        ("role".into(), HostValue::string("tool")),
                    ]
                    .into_iter()
                    .collect(),
                )],
            )
            .expect("create window");
        let created = created.as_object().expect("window descriptor");
        let id = created.get("id").and_then(HostValue::as_f64).unwrap() as u64;
        let mount_root = created
            .get("mountRoot")
            .and_then(HostValue::as_f64)
            .unwrap() as u64;
        assert_eq!(id, 1);
        assert_eq!(mount_root, NODE_HANDLE_DOCUMENT_STRIDE + 2);
        assert_eq!(
            runtime
                .host(VueWindowId(id))
                .unwrap()
                .lock()
                .unwrap()
                .document()
                .lock()
                .unwrap()
                .stylesheet_count(),
            1,
            "late-created windows must inherit application stylesheets"
        );

        let element = api
            .call(
                "windowCall",
                &[
                    HostValue::Number(id as f64),
                    HostValue::string("createElement"),
                    HostValue::Array(vec![HostValue::string("section")]),
                ],
            )
            .expect("create auxiliary element")
            .as_f64()
            .unwrap() as u64;
        assert!(element > NODE_HANDLE_DOCUMENT_STRIDE);
        api.call(
            "windowCall",
            &[
                HostValue::Number(id as f64),
                HostValue::string("insert"),
                HostValue::Array(vec![
                    HostValue::Number(element as f64),
                    HostValue::Number(mount_root as f64),
                    HostValue::Null,
                ]),
            ],
        )
        .expect("insert auxiliary element");

        assert_eq!(runtime.window_ids(), [VueWindowId::PRIMARY, VueWindowId(1)]);
        let snapshot = runtime
            .semantic_snapshot(VueWindowId(1))
            .expect("auxiliary snapshot");
        assert!(snapshot.widgets.iter().any(|widget| widget.id == element));
        assert!(matches!(
            runtime.drain_window_commands().as_slice(),
            [VueWindowCommand::Open { id: VueWindowId(1), options }]
                if options.title == "Tool" && options.role == VueWindowRole::Tool
        ));
    }

    #[test]
    fn close_keeps_document_until_native_confirmation() {
        let runtime = VueRuntime::default();
        let api = runtime.host_api_registry();
        let id = api
            .call("windowCreate", &[])
            .unwrap()
            .as_object()
            .unwrap()
            .get("id")
            .and_then(HostValue::as_f64)
            .unwrap() as u64;
        runtime.drain_window_commands();
        api.call("windowClose", &[HostValue::Number(id as f64)])
            .expect("request close");
        assert!(runtime.host(VueWindowId(id)).is_some());
        assert_eq!(
            runtime.drain_window_commands(),
            [VueWindowCommand::Close(VueWindowId(id))]
        );
        runtime
            .notify_window_closed(VueWindowId(id))
            .expect("confirm close");
        assert!(runtime.host(VueWindowId(id)).is_none());
    }

    #[test]
    fn local_storage_is_shared_but_session_storage_is_window_local() {
        let runtime = VueRuntime::default();
        let api = runtime.host_api_registry();
        let id = api
            .call("windowCreate", &[])
            .unwrap()
            .as_object()
            .unwrap()
            .get("id")
            .and_then(HostValue::as_f64)
            .unwrap() as u64;

        api.call(
            "storageSet",
            &[
                HostValue::string("local"),
                HostValue::string("shared"),
                HostValue::string("primary"),
            ],
        )
        .unwrap();
        api.call(
            "storageSet",
            &[
                HostValue::string("session"),
                HostValue::string("private"),
                HostValue::string("primary"),
            ],
        )
        .unwrap();

        let local = api
            .call(
                "windowCall",
                &[
                    HostValue::Number(id as f64),
                    HostValue::string("storageGet"),
                    HostValue::Array(vec![
                        HostValue::string("local"),
                        HostValue::string("shared"),
                    ]),
                ],
            )
            .unwrap();
        let session = api
            .call(
                "windowCall",
                &[
                    HostValue::Number(id as f64),
                    HostValue::string("storageGet"),
                    HostValue::Array(vec![
                        HostValue::string("session"),
                        HostValue::string("private"),
                    ]),
                ],
            )
            .unwrap();

        assert_eq!(local.as_str(), Some("primary"));
        assert!(matches!(session, HostValue::Null));
    }

    #[test]
    fn window_chrome_commands_are_queued_and_geometry_is_readable() {
        let runtime = VueRuntime::default();
        let api = runtime.host_api_registry();
        api.call(
            "windowSetBounds",
            &[
                HostValue::Number(0.0),
                HostValue::Number(12.0),
                HostValue::Number(24.0),
                HostValue::Number(640.0),
                HostValue::Number(480.0),
            ],
        )
        .expect("set bounds");
        api.call(
            "windowSetFullscreen",
            &[HostValue::Number(0.0), HostValue::Bool(true)],
        )
        .expect("set fullscreen");
        let geometry = api
            .call("windowGeometry", &[HostValue::Number(0.0)])
            .unwrap()
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(geometry.get("x").and_then(HostValue::as_f64), Some(12.0));
        assert_eq!(
            geometry.get("width").and_then(HostValue::as_f64),
            Some(640.0)
        );
        assert_eq!(
            geometry.get("fullscreen").and_then(HostValue::as_bool),
            Some(true)
        );
        let commands = runtime.drain_window_commands();
        assert!(commands.iter().any(|command| matches!(
            command,
            VueWindowCommand::SetBounds {
                id: VueWindowId(0),
                width,
                ..
            } if (*width - 640.0).abs() < f64::EPSILON
        )));
        assert!(commands.iter().any(|command| matches!(
            command,
            VueWindowCommand::SetFullscreen {
                id: VueWindowId(0),
                fullscreen: true
            }
        )));
    }

    #[cfg(feature = "iced-view")]
    #[test]
    fn native_component_render_failure_keeps_identity_and_structured_error() {
        let runtime = VueRuntime::default();
        let sender = HostEventSender::with_capacity(2);
        runtime.state.lock().unwrap().events = Some(sender.clone());
        let node = runtime
            .host(VueWindowId::PRIMARY)
            .unwrap()
            .lock()
            .unwrap()
            .mount_root();
        runtime.components().report_error(
            "live2d-view",
            node.0,
            JsException::new("draw failed")
                .with_name("NativeComponentRenderError")
                .with_code("render_failed")
                .with_details(HostValue::Object(
                    [("frame".into(), HostValue::Number(7.0))]
                        .into_iter()
                        .collect(),
                )),
        );

        assert_eq!(runtime.flush_native_component_failures().unwrap(), 1);
        let events = sender.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "native-component-error");
        let payload = events[0].payload.as_object().unwrap();
        assert_eq!(
            payload.get("windowId").and_then(HostValue::as_f64),
            Some(0.0)
        );
        assert_eq!(
            payload.get("component").and_then(HostValue::as_str),
            Some("live2d-view")
        );
        let error = payload.get("error").and_then(HostValue::as_object).unwrap();
        assert_eq!(
            error.get("code").and_then(HostValue::as_str),
            Some("render_failed")
        );
    }

    #[cfg(feature = "iced-view")]
    #[test]
    fn native_component_failure_is_retried_when_reliable_queue_is_full() {
        let runtime = VueRuntime::default();
        let sender = HostEventSender::with_capacity(1);
        sender
            .send_reliable("window-ready", HostValue::Null)
            .unwrap();
        runtime.state.lock().unwrap().events = Some(sender.clone());
        let node = runtime
            .host(VueWindowId::PRIMARY)
            .unwrap()
            .lock()
            .unwrap()
            .mount_root();
        runtime.components().report_error(
            "live2d-view",
            node.0,
            JsException::new("draw failed").with_code("render_failed"),
        );

        assert!(runtime.flush_native_component_failures().is_err());
        assert!(runtime.has_native_component_failures());
        sender.drain();
        assert_eq!(runtime.flush_native_component_failures().unwrap(), 1);
        assert_eq!(sender.drain()[0].name, "native-component-error");
    }
}
