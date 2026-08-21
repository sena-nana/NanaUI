//! Production controller joining one JS engine, all Vue documents, and the
//! NanaUI Scene/`run_runtime` host.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use nana_js_engine::{HostApiRegistry, JsEngine, JsEngineError, RuntimeArtifact};
use nana_ui::{
    HostTextureRegistry, HostedGpuResources, RuntimeProgram, RuntimeProgramContext,
    RuntimeProgramUpdate, RuntimeRedraw, RuntimeWindowSettings, ThemeMode, window_material_effect,
};
use nana_ui_platform::{InputEvent, PointerPhase, WindowEvent, WindowGeometry, WindowId};
use nana_ui_runtime::FrameworkError;
use nana_ui_scene::RuntimeDocument;

use crate::{
    BridgeEvent, FileDragEventKind, HostedInputResult, InputModifiers, KeyboardEventKind,
    KeyboardInput, PointerEventKind, PointerInput, PointerType, SharedRuntimeDocument, VueRuntime,
    VueWindowId, WheelInput, WindowLifecycleEvent,
};

thread_local! {
    static PENDING_VUE_BOOTSTRAP: RefCell<Option<Box<dyn Any>>> = RefCell::new(None);
}

/// A single-engine Vue runtime suitable for embedding in `RuntimeProgram`.
pub struct VueHostedRuntime<E: JsEngine> {
    engine: E,
    vue: VueRuntime,
    application_api: HostApiRegistry,
}

impl<E: JsEngine> VueHostedRuntime<E> {
    pub fn new(
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
    ) -> Result<Self, JsEngineError> {
        let mut runtime = Self {
            engine,
            vue: VueRuntime::new(physical_width, physical_height, scale_factor),
            application_api,
        };
        runtime
            .vue
            .initialize(&mut runtime.engine, artifact, &runtime.application_api)?;
        Ok(runtime)
    }

    pub fn vue(&self) -> &VueRuntime {
        &self.vue
    }

    pub fn engine(&self) -> &E {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut E {
        &mut self.engine
    }

    pub fn components(&self) -> crate::NativeComponentRegistry {
        self.vue.components()
    }

    pub fn inject_theme(&mut self, theme: ThemeMode) -> Result<(), JsEngineError> {
        let host = self.require_host(VueWindowId::PRIMARY)?;
        host.lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .inject_theme(&mut self.engine, theme)
    }

    pub fn inject_stylesheet(&self, css: &str) -> Result<(), JsEngineError> {
        self.vue.inject_stylesheet(css)
    }

    pub fn bind_host_gpu(&mut self, resources: HostedGpuResources) -> Result<u64, JsEngineError> {
        let generation = self.vue.bind_host_gpu(resources)?;
        self.register_complete_host_api()?;
        Ok(generation)
    }

    pub fn dispatch_bridge_event(
        &mut self,
        _id: WindowId,
        event: BridgeEvent,
    ) -> Result<bool, JsEngineError> {
        let document = crate::DocumentId::from_node(crate::NodeHandle(event.widget_id()));
        let id = VueWindowId(document.0.saturating_sub(1));
        let host = self
            .vue
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        host.lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .dispatch_bridge_event(&mut self.engine, event)
    }

    pub fn accessibility_action(
        &mut self,
        id: WindowId,
        request: nana_ui_runtime::AccessibilityActionRequest,
    ) -> Result<bool, JsEngineError> {
        let host = self.require_host(VueWindowId(id.0))?;
        let mut host = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?;
        let target = crate::NodeHandle(request.target.get());
        match request.action {
            nana_ui_runtime::AccessibilityAction::Focus => {
                host.accessibility_focus(&mut self.engine, target)
            }
            nana_ui_runtime::AccessibilityAction::Click => {
                host.accessibility_click(&mut self.engine, target)
            }
            nana_ui_runtime::AccessibilityAction::SetValue(value) => {
                host.accessibility_set_value(&mut self.engine, target, &value)
            }
            nana_ui_runtime::AccessibilityAction::SetSelection(selection) => {
                host.accessibility_set_selection(&mut self.engine, target, selection)
            }
        }
    }

    pub fn accessibility_snapshot(&self, id: WindowId) -> Vec<nana_ui_runtime::AccessibilityNode> {
        self.vue
            .host(VueWindowId(id.0))
            .and_then(|host| host.lock().ok().map(|host| host.document()))
            .and_then(|document| {
                document
                    .lock()
                    .ok()
                    .map(|document| document.accessibility_snapshot())
            })
            .unwrap_or_default()
    }

    pub fn take_accessibility_update(
        &mut self,
        id: WindowId,
    ) -> Option<nana_ui_runtime::AccessibilityUpdate> {
        self.vue
            .host(VueWindowId(id.0))
            .and_then(|host| host.lock().ok().map(|host| host.document()))
            .and_then(|document| {
                document
                    .lock()
                    .ok()
                    .and_then(|mut document| document.take_accessibility_update())
            })
    }

    fn runtime_program_update(&self, redraw: bool) -> RuntimeProgramUpdate {
        let window_commands = self.vue.drain_runtime_window_commands();
        RuntimeProgramUpdate {
            redraw: if redraw {
                RuntimeRedraw::All
            } else {
                RuntimeRedraw::None
            },
            window_commands,
            exit: false,
        }
    }

    pub fn runtime_input(
        &mut self,
        id: WindowId,
        event: &InputEvent,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        match self.emit_runtime_input(VueWindowId(id.0), event) {
            Ok(_) => Ok(self.runtime_program_update(true)),
            Err(_) => Ok(RuntimeProgramUpdate::default()),
        }
    }

    fn emit_runtime_input(
        &mut self,
        id: VueWindowId,
        event: &InputEvent,
    ) -> Result<HostedInputResult, JsEngineError> {
        let host = self.require_host(id)?;
        let mut host = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?;
        match event {
            InputEvent::Pointer {
                phase,
                pointer_id,
                pointer_type,
                x,
                y,
                screen_x,
                screen_y,
                button,
                buttons,
                pressure,
                tangential_pressure,
                tilt_x,
                tilt_y,
                twist,
                is_primary,
                modifiers,
            } => {
                let kind = match phase {
                    PointerPhase::Down => PointerEventKind::Down,
                    PointerPhase::Move => PointerEventKind::Move,
                    PointerPhase::Up => PointerEventKind::Up,
                    PointerPhase::Cancel => PointerEventKind::Cancel,
                };
                host.emit_pointer_from_runtime(
                    &mut self.engine,
                    PointerInput {
                        kind,
                        pointer_id: *pointer_id,
                        pointer_type: match pointer_type {
                            nana_ui_platform::PointerType::Mouse => PointerType::Mouse,
                            nana_ui_platform::PointerType::Touch => PointerType::Touch,
                            nana_ui_platform::PointerType::Pen => PointerType::Pen,
                        },
                        is_primary: *is_primary,
                        client_x: *x,
                        client_y: *y,
                        screen_x: *screen_x,
                        screen_y: *screen_y,
                        button: *button,
                        buttons: *buttons,
                        pressure: *pressure,
                        tangential_pressure: *tangential_pressure,
                        tilt_x: *tilt_x,
                        tilt_y: *tilt_y,
                        twist: *twist,
                        modifiers: InputModifiers {
                            alt: modifiers.alt,
                            control: modifiers.control,
                            meta: modifiers.meta,
                            shift: modifiers.shift,
                        },
                    },
                )
            }
            InputEvent::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                line_delta,
                modifiers,
            } => host.emit_wheel_from_runtime(
                &mut self.engine,
                WheelInput {
                    client_x: *x,
                    client_y: *y,
                    screen_x: *x,
                    screen_y: *y,
                    delta_x: *delta_x,
                    delta_y: *delta_y,
                    delta_mode: u8::from(*line_delta),
                    modifiers: InputModifiers {
                        alt: modifiers.alt,
                        control: modifiers.control,
                        meta: modifiers.meta,
                        shift: modifiers.shift,
                    },
                },
            ),
            InputEvent::Keyboard {
                pressed,
                key,
                text,
                code,
                repeat,
                modifiers,
            } => {
                let allowed = host.emit_keyboard_from_runtime(
                    &mut self.engine,
                    &KeyboardInput {
                        kind: if *pressed {
                            KeyboardEventKind::Down
                        } else {
                            KeyboardEventKind::Up
                        },
                        key: key.clone(),
                        code: code.clone(),
                        location: 0,
                        repeat: *repeat,
                        composing: false,
                        modifiers: InputModifiers {
                            alt: modifiers.alt,
                            control: modifiers.control,
                            meta: modifiers.meta,
                            shift: modifiers.shift,
                        },
                    },
                    None,
                )?;
                if *pressed
                    && let Some(text) = text.as_deref().filter(|text| !text.is_empty())
                    && let Some(target) = host.focused()
                {
                    let is_text = host
                        .document()
                        .lock()
                        .ok()
                        .is_some_and(|document| document.text_input_state(target).is_some());
                    if is_text {
                        let _ = host.emit_text_events_from_runtime(
                            &mut self.engine,
                            target,
                            text,
                            "insertText",
                        );
                    }
                }
                Ok(HostedInputResult {
                    targeted: true,
                    default_prevented: !allowed,
                    consumed: !allowed,
                })
            }
        }
    }

    pub fn runtime_window_event(&mut self, event: WindowEvent) -> RuntimeProgramUpdate {
        let close_primary = matches!(
            event,
            WindowEvent::CloseRequested {
                id: WindowId::PRIMARY,
            }
        );
        if let Err(_error) = self.handle_platform_window_event(event) {
            return RuntimeProgramUpdate::default();
        }
        if close_primary {
            return RuntimeProgramUpdate::exit();
        }
        self.runtime_program_update(true)
    }

    fn handle_platform_window_event(&mut self, event: WindowEvent) -> Result<(), JsEngineError> {
        match event {
            WindowEvent::Ready { id, geometry } => {
                let id = VueWindowId(id.0);
                self.vue.set_viewport(
                    id,
                    geometry.physical_size.0.max(1),
                    geometry.physical_size.1.max(1),
                    geometry.scale_factor.max(0.01),
                )?;
                self.vue.record_platform_geometry(id, &geometry)?;
                self.vue.bind_window(&mut self.engine, id)?;
                self.vue.notify_window_ready(id)?;
                self.vue.pump_lifecycle(
                    &mut self.engine,
                    id,
                    WindowLifecycleEvent::ResizeWithScale {
                        width: geometry.logical_size.0 as f64,
                        height: geometry.logical_size.1 as f64,
                        scale_factor: geometry.scale_factor as f64,
                    },
                )?;
            }
            WindowEvent::Resized { id, geometry } => {
                let id = VueWindowId(id.0);
                self.vue.set_viewport(
                    id,
                    geometry.physical_size.0.max(1),
                    geometry.physical_size.1.max(1),
                    geometry.scale_factor.max(0.01),
                )?;
                self.vue.record_platform_geometry(id, &geometry)?;
                self.vue.pump_lifecycle(
                    &mut self.engine,
                    id,
                    WindowLifecycleEvent::ResizeWithScale {
                        width: geometry.logical_size.0 as f64,
                        height: geometry.logical_size.1 as f64,
                        scale_factor: geometry.scale_factor as f64,
                    },
                )?;
            }
            WindowEvent::VisibilityChanged { id, hidden } => {
                self.vue.pump_lifecycle(
                    &mut self.engine,
                    VueWindowId(id.0),
                    WindowLifecycleEvent::VisibilityChange { hidden },
                )?;
            }
            WindowEvent::FocusChanged { id, focused } => {
                self.vue.pump_lifecycle(
                    &mut self.engine,
                    VueWindowId(id.0),
                    if focused {
                        WindowLifecycleEvent::Focus
                    } else {
                        WindowLifecycleEvent::Blur
                    },
                )?;
            }
            WindowEvent::Ime { id, event } => {
                let host = self.require_host(VueWindowId(id.0))?;
                host.lock()
                    .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                    .emit_native_ime_from_runtime(&mut self.engine, &event)?;
            }
            WindowEvent::Closed { id } => {
                self.vue.notify_window_closed(VueWindowId(id.0))?;
            }
            WindowEvent::Moved { id, geometry } => {
                self.vue
                    .record_platform_geometry(VueWindowId(id.0), &geometry)?;
            }
            WindowEvent::CloseRequested { id } => {
                if id != WindowId::PRIMARY {
                    self.vue.request_close(VueWindowId(id.0))?;
                }
            }
            WindowEvent::FileHovered {
                id,
                paths,
                position,
            } => self.emit_file_drag(id, FileDragEventKind::Hover, &paths, position)?,
            WindowEvent::FileDropped {
                id,
                paths,
                position,
            } => self.emit_file_drag(id, FileDragEventKind::Drop, &paths, position)?,
            WindowEvent::FileHoverCancelled { id } => {
                self.emit_file_drag(id, FileDragEventKind::Cancel, &[], None)?
            }
        }
        Ok(())
    }

    fn emit_file_drag(
        &mut self,
        id: WindowId,
        kind: FileDragEventKind,
        paths: &[PathBuf],
        position: Option<(f32, f32)>,
    ) -> Result<(), JsEngineError> {
        self.require_host(VueWindowId(id.0))?
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .dispatch_file_drag(&mut self.engine, kind, paths, position)?;
        Ok(())
    }

    pub fn runtime_accessibility_action(
        &mut self,
        id: WindowId,
        request: nana_ui_runtime::AccessibilityActionRequest,
    ) -> Result<RuntimeProgramUpdate, JsEngineError> {
        let changed = self.accessibility_action(WindowId(id.0), request)?;
        Ok(self.runtime_program_update(changed))
    }

    pub fn runtime_rebuild_gpu(&mut self, resources: HostedGpuResources) -> RuntimeProgramUpdate {
        match self
            .vue
            .replace_host_gpu(&mut self.engine, resources, "hosted GPU device recovered")
        {
            Ok(_) => {
                let _ = self.register_complete_host_api();
                self.runtime_program_update(true)
            }
            Err(_) => RuntimeProgramUpdate::default(),
        }
    }

    pub fn runtime_wake(&mut self) -> RuntimeProgramUpdate {
        match self.pump() {
            Ok(work) if work > 0 => self.runtime_program_update(true),
            _ => self.runtime_program_update(false),
        }
    }

    pub fn prepare_runtime_window(&self, id: WindowId) {
        let Some(host) = self.vue.host(VueWindowId(id.0)) else {
            return;
        };
        let Ok(mut host) = host.lock() else {
            return;
        };
        host.prepare_canvas_gpu();
        let snapshot = host.semantic_snapshot();
        if let Ok(mut document) = host.document().lock() {
            document.sync_semantic_styles(&snapshot);
        }
        host.resolve_layout();
    }

    pub fn host_textures_for(&self, id: WindowId) -> Option<HostTextureRegistry> {
        let host = self.vue.host(VueWindowId(id.0))?;
        host.lock().ok().map(|host| host.host_textures().clone())
    }

    pub fn shared_runtime_document(&self, id: WindowId) -> Option<Arc<SharedRuntimeDocument>> {
        self.vue.shared_runtime_document(VueWindowId(id.0))
    }

    pub fn handle_window_event(&mut self, event: WindowEvent) -> Result<(), JsEngineError> {
        self.handle_platform_window_event(event)
    }

    pub fn drain_window_commands(&self) -> Vec<crate::VueWindowCommand> {
        self.vue.drain_window_commands()
    }

    pub fn next_wakeup(&self) -> Option<std::time::Instant> {
        if self.vue.has_native_component_failures() {
            return Some(std::time::Instant::now());
        }
        self.vue.next_wakeup()
    }

    pub fn pump(&mut self) -> Result<usize, JsEngineError> {
        let failures = self.vue.flush_native_component_failures()?;
        let ids = self.vue.window_ids();
        let mut work = failures;
        for id in ids {
            work += self.vue.pump_frame(&mut self.engine, id)?;
        }
        Ok(work)
    }

    fn require_host(
        &self,
        id: VueWindowId,
    ) -> Result<std::sync::Arc<std::sync::Mutex<crate::VueHost>>, JsEngineError> {
        self.vue
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))
    }

    fn register_complete_host_api(&mut self) -> Result<(), JsEngineError> {
        let mut api = self.vue.host_api_registry();
        api.try_extend(&self.application_api)?;
        self.engine.register_host_api(&api)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostedTextPosition {
    line: usize,
    index: usize,
}

#[cfg(test)]
fn hosted_text_position(value: &str, byte_offset: usize) -> Option<HostedTextPosition> {
    if byte_offset > value.len() || !value.is_char_boundary(byte_offset) {
        return None;
    }
    let before = &value[..byte_offset];
    Some(HostedTextPosition {
        line: before.bytes().filter(|byte| *byte == b'\n').count(),
        index: before
            .rsplit_once('\n')
            .map_or(before.len(), |(_, line)| line.len()),
    })
}

/// Vue application as a [`RuntimeProgram`].
///
/// Owns one [`VueHostedRuntime`] and enters `run_runtime` / `SceneWgpuPainter`.
pub struct VueRuntimeProgram<E: JsEngine> {
    runtime: VueHostedRuntime<E>,
    documents: HashMap<WindowId, Arc<SharedRuntimeDocument>>,
    theme: ThemeMode,
}

/// Historical name for [`VueRuntimeProgram`].
pub type VueHostedProgram<E> = VueRuntimeProgram<E>;

impl<E: JsEngine> VueRuntimeProgram<E> {
    pub fn bootstrap(
        context: &RuntimeProgramContext<BridgeEvent>,
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
    ) -> Result<Self, JsEngineError> {
        let geometry = context.geometry();
        Self::bootstrap_from_gpu(
            context.gpu().clone(),
            geometry.physical_size.0.max(1),
            geometry.physical_size.1.max(1),
            geometry.scale_factor.max(0.01),
            Some(geometry),
            engine,
            artifact,
            application_api,
        )
    }

    fn bootstrap_from_gpu(
        gpu: HostedGpuResources,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        platform_geometry: Option<WindowGeometry>,
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
    ) -> Result<Self, JsEngineError> {
        let mut runtime = VueHostedRuntime::new(
            engine,
            artifact,
            application_api,
            physical_width,
            physical_height,
            scale_factor,
        )?;
        runtime.bind_host_gpu(gpu)?;
        if let Some(geometry) = platform_geometry {
            runtime
                .vue
                .record_platform_geometry(VueWindowId::PRIMARY, &geometry)?;
        }
        let _ = runtime.inject_theme(ThemeMode::Light);
        let mut program = Self {
            runtime,
            documents: HashMap::new(),
            theme: ThemeMode::Light,
        };
        program.sync_documents();
        Ok(program)
    }

    pub fn from_runtime(runtime: VueHostedRuntime<E>) -> Self {
        let mut program = Self {
            runtime,
            documents: HashMap::new(),
            theme: ThemeMode::Light,
        };
        program.sync_documents();
        program
    }

    pub fn runtime(&self) -> &VueHostedRuntime<E> {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut VueHostedRuntime<E> {
        &mut self.runtime
    }

    fn sync_documents(&mut self) {
        let ids = self.runtime.vue.window_ids();
        let live = ids
            .iter()
            .map(|id| WindowId(id.0))
            .collect::<std::collections::HashSet<_>>();
        for id in ids {
            let window = WindowId(id.0);
            if self.documents.contains_key(&window) {
                continue;
            }
            if let Some(document) = self.runtime.shared_runtime_document(window) {
                self.documents.insert(window, document);
            }
        }
        self.documents.retain(|id, _| live.contains(id));
    }
}

impl<E: JsEngine + 'static> VueRuntimeProgram<E> {
    /// Production entry for caller-owned engines. Release applications pass a
    /// `nana_js_v8::V8Engine` here, keeping one engine for every Vue window.
    pub fn run(
        settings: RuntimeWindowSettings,
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
    ) -> Result<(), nana_ui::HostedRunError> {
        PENDING_VUE_BOOTSTRAP.with(|slot| {
            *slot.borrow_mut() = Some(Box::new((engine, artifact, application_api)));
        });
        nana_ui::run_runtime::<Self>(settings)
    }
}

impl<E: JsEngine + 'static> RuntimeProgram for VueRuntimeProgram<E> {
    type Message = BridgeEvent;
    type Error = JsEngineError;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let (engine, artifact, application_api) = PENDING_VUE_BOOTSTRAP
            .with(|slot| slot.borrow_mut().take())
            .and_then(|boxed| {
                boxed
                    .downcast::<(E, RuntimeArtifact, HostApiRegistry)>()
                    .ok()
            })
            .map(|boxed| *boxed)
            .ok_or_else(|| {
                JsEngineError::new(
                    "VueRuntimeProgram::run must supply the engine and runtime artifact",
                )
            })?;
        Self::bootstrap(context, engine, artifact, application_api)
            .map(|program| (program, Vec::new()))
    }

    fn document(&self, id: WindowId) -> Option<&RuntimeDocument> {
        self.documents.get(&id).map(|document| document.get())
    }

    fn document_mut(&mut self, id: WindowId) -> Option<&mut RuntimeDocument> {
        self.sync_documents();
        self.documents.get(&id).map(|document| document.get_mut())
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.sync_documents();
        match self
            .runtime
            .dispatch_bridge_event(WindowId::PRIMARY, message)
        {
            Ok(_) => {
                self.sync_documents();
                self.runtime.runtime_program_update(true)
            }
            Err(_) => RuntimeProgramUpdate::default(),
        }
    }

    fn theme_mode(&self) -> ThemeMode {
        self.theme
    }

    fn window_material_mode(&self) -> nana_ui::MaterialEffect {
        self.runtime
            .vue()
            .host(VueWindowId::PRIMARY)
            .and_then(|host| {
                host.lock()
                    .ok()
                    .map(|guard| window_material_effect(guard.appearance().window_material()))
            })
            .unwrap_or(nana_ui::MaterialEffect::Solid)
    }

    fn host_textures(&self, id: WindowId) -> Option<HostTextureRegistry> {
        self.runtime.host_textures_for(id)
    }

    fn prepare_window_frame(
        &mut self,
        id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) {
        self.sync_documents();
        self.runtime.prepare_runtime_window(id);
    }

    fn take_accessibility_update(
        &mut self,
        id: WindowId,
    ) -> Option<nana_ui_runtime::AccessibilityUpdate> {
        self.sync_documents();
        self.runtime.take_accessibility_update(id)
    }

    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        let _ = self.runtime.runtime_rebuild_gpu(context.gpu().clone());
    }

    fn input_event(
        &mut self,
        id: WindowId,
        event: &InputEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.sync_documents();
        let update = self.runtime.runtime_input(id, event)?;
        self.sync_documents();
        Ok(update)
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.sync_documents();
        let update = self.runtime.runtime_window_event(event);
        self.sync_documents();
        update
    }

    fn next_wakeup(&self) -> Option<Instant> {
        self.runtime.next_wakeup()
    }

    fn wake(
        &mut self,
        _now: Instant,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.sync_documents();
        let update = self.runtime.runtime_wake();
        self.sync_documents();
        update
    }

    fn accessibility_action(
        &mut self,
        id: WindowId,
        request: nana_ui_runtime::AccessibilityActionRequest,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.sync_documents();
        Ok(self
            .runtime
            .runtime_accessibility_action(id, request)
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VueHost;

    #[test]
    fn vue_window_documents_are_the_same_runtime_tree() {
        let host = VueHost::new();
        let shared = host.shared_runtime_document();
        let from_tree = host.document().lock().unwrap().shared_runtime_document();
        assert!(Arc::ptr_eq(&shared, &from_tree));
        assert_eq!(
            shared.get().document(),
            nana_ui_runtime::DocumentId::new(1).unwrap()
        );
    }

    #[test]
    fn hosted_text_positions_preserve_utf8_lines_and_byte_indices() {
        let value = "你a\n好b";
        assert_eq!(
            hosted_text_position(value, "你".len()),
            Some(HostedTextPosition { line: 0, index: 3 })
        );
        assert_eq!(
            hosted_text_position(value, "你a\n好".len()),
            Some(HostedTextPosition { line: 1, index: 3 })
        );
        assert_eq!(hosted_text_position(value, 1), None);
    }
}
