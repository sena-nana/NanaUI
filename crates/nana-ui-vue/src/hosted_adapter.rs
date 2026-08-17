//! Production controller joining one JS engine, all Vue documents, and the
//! NanaUI hosted window/GPU runtime.

use std::time::Instant;

use iced::Element;
use nana_js_engine::{HostApiRegistry, JsEngine, JsEngineError, RuntimeArtifact};
use nana_ui::{
    AppearanceSettings, BackdropTarget, HostedGpuResources, HostedInputDisposition,
    HostedInputEvent, HostedPointerPhase, HostedPointerType, HostedProgram, HostedProgramContext,
    HostedProgramUpdate, HostedRuntimeEvent, HostedTextPosition, HostedUiCommand,
    HostedWindowCommand, HostedWindowEvent, HostedWindowId, ThemeMode, WindowMaterialMode,
};

use crate::iced_app::view_semantic_tree_static_with_scene;
use crate::{
    BridgeEvent, FileDragEventKind, HostedInputResult, InputModifiers, KeyboardEventKind,
    KeyboardInput, PointerEventKind, PointerInput, PointerType, VueRuntime, VueWindowId,
    WheelInput, WindowLifecycleEvent, theme_tokens_from_snapshot,
};

/// A single-engine Vue runtime suitable for embedding in `HostedProgram`.
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

    pub fn view_window(
        &self,
        id: HostedWindowId,
        native_material: bool,
    ) -> Result<Element<'static, BridgeEvent>, JsEngineError> {
        let id = VueWindowId(id.0);
        let host = self
            .vue
            .host(id)
            .ok_or_else(|| JsEngineError::new(format!("unknown Vue window {}", id.0)))?;
        let mut host = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?;
        host.prepare_editors();
        host.prepare_menus();
        host.prepare_canvas_gpu();
        let snapshot = host.semantic_snapshot();
        let document = host.document();
        let document = document
            .lock()
            .map_err(|_| JsEngineError::new("Vue document poisoned"))?;
        let viewport = document.logical_size();
        let tokens = theme_tokens_from_snapshot(&snapshot, native_material);
        Ok(view_semantic_tree_static_with_scene(
            &snapshot,
            tokens,
            Some(viewport),
            Some(host.editors()),
            Some(host.menus()),
            Some(host.host_textures()),
            Some(host.canvas_runtime_ref()),
            Some(host.components()),
            Some(document.scene()),
            Some(host.layout_box_store()),
            |event| event,
        ))
    }

    pub fn dispatch_bridge_event(
        &mut self,
        _id: HostedWindowId,
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
        id: HostedWindowId,
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

    pub fn accessibility_snapshot(
        &self,
        id: HostedWindowId,
    ) -> Vec<nana_ui_runtime::AccessibilityNode> {
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
        id: HostedWindowId,
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

    pub fn hosted_accessibility_action(
        &mut self,
        id: HostedWindowId,
        request: nana_ui_runtime::AccessibilityActionRequest,
    ) -> Result<HostedProgramUpdate, JsEngineError> {
        let selects_text = matches!(
            &request.action,
            nana_ui_runtime::AccessibilityAction::SetSelection(_)
        );
        let focuses = matches!(&request.action, nana_ui_runtime::AccessibilityAction::Focus);
        let target = request.target;
        let changed = self.accessibility_action(id, request)?;
        let mut update = self.program_update(changed);
        if changed {
            if focuses && let Some(command) = self.text_focus_command(id, target)? {
                update = update.with_ui_commands([command]);
            } else if selects_text && let Some(command) = self.text_selection_command(id, target)? {
                update = update.with_ui_commands([command]);
            }
        }
        Ok(update)
    }

    fn text_focus_command(
        &self,
        id: HostedWindowId,
        target: nana_ui_runtime::StableNodeId,
    ) -> Result<Option<HostedUiCommand>, JsEngineError> {
        let host = self.require_host(VueWindowId(id.0))?;
        let document = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .document();
        let is_text_input = document
            .lock()
            .map_err(|_| JsEngineError::new("Vue document poisoned"))?
            .text_input_state(crate::NodeHandle(target.get()))
            .is_some();
        Ok(is_text_input.then(|| HostedUiCommand::Focus {
            window_id: id,
            target: crate::iced_app::hosted_text_widget_id(target.get()),
        }))
    }

    fn text_selection_command(
        &self,
        id: HostedWindowId,
        target: nana_ui_runtime::StableNodeId,
    ) -> Result<Option<HostedUiCommand>, JsEngineError> {
        let host = self.require_host(VueWindowId(id.0))?;
        let document = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .document();
        let Some(state) = document
            .lock()
            .map_err(|_| JsEngineError::new("Vue document poisoned"))?
            .text_input_state(crate::NodeHandle(target.get()))
        else {
            // A select listener may synchronously remove its target. The
            // Runtime/Vue mutation remains valid; there is no retained editor
            // left to synchronize after the callback.
            return Ok(None);
        };
        let anchor = hosted_text_position(&state.value, state.selection.anchor)
            .ok_or_else(|| JsEngineError::new("invalid accessibility selection anchor"))?;
        let focus = hosted_text_position(&state.value, state.selection.focus)
            .ok_or_else(|| JsEngineError::new("invalid accessibility selection focus"))?;
        Ok(Some(HostedUiCommand::SelectText {
            window_id: id,
            target: crate::iced_app::hosted_text_widget_id(target.get()),
            anchor,
            focus,
        }))
    }

    pub fn dispatch_input(
        &mut self,
        id: HostedWindowId,
        event: HostedInputEvent,
    ) -> Result<HostedInputResult, JsEngineError> {
        let id = VueWindowId(id.0);
        match event {
            HostedInputEvent::Pointer {
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
                    HostedPointerPhase::Down => PointerEventKind::Down,
                    HostedPointerPhase::Move => PointerEventKind::Move,
                    HostedPointerPhase::Up => PointerEventKind::Up,
                    HostedPointerPhase::Cancel => PointerEventKind::Cancel,
                };
                let input = PointerInput {
                    kind,
                    pointer_id,
                    pointer_type: match pointer_type {
                        HostedPointerType::Mouse => PointerType::Mouse,
                        HostedPointerType::Touch => PointerType::Touch,
                        HostedPointerType::Pen => PointerType::Pen,
                    },
                    is_primary,
                    client_x: x,
                    client_y: y,
                    screen_x,
                    screen_y,
                    button,
                    buttons,
                    pressure,
                    tangential_pressure,
                    tilt_x,
                    tilt_y,
                    twist,
                    modifiers: map_modifiers(modifiers),
                };
                let host = self.require_host(id)?;
                let result = host
                    .lock()
                    .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                    .dispatch_pointer_result(&mut self.engine, input)?;
                Ok(result)
            }
            HostedInputEvent::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                line_delta,
                modifiers,
            } => {
                let host = self.require_host(id)?;
                let result = host
                    .lock()
                    .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                    .dispatch_wheel_result(
                        &mut self.engine,
                        WheelInput {
                            client_x: x,
                            client_y: y,
                            screen_x: x,
                            screen_y: y,
                            delta_x,
                            delta_y,
                            delta_mode: u8::from(line_delta),
                            modifiers: map_modifiers(modifiers),
                        },
                    )?;
                Ok(result)
            }
            HostedInputEvent::Keyboard {
                pressed,
                key,
                text: _,
                code,
                repeat,
                modifiers,
            } => {
                let host = self.require_host(id)?;
                let allowed = host
                    .lock()
                    .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                    .dispatch_keyboard(
                        &mut self.engine,
                        &KeyboardInput {
                            kind: if pressed {
                                KeyboardEventKind::Down
                            } else {
                                KeyboardEventKind::Up
                            },
                            key,
                            code,
                            location: 0,
                            repeat,
                            composing: false,
                            modifiers: map_modifiers(modifiers),
                        },
                        None,
                    )?;
                Ok(HostedInputResult {
                    targeted: true,
                    default_prevented: !allowed,
                    consumed: !allowed,
                })
            }
        }
    }

    /// Dispatch raw host input and decide whether Iced should also see the event.
    pub fn hosted_input(
        &mut self,
        id: HostedWindowId,
        event: HostedInputEvent,
    ) -> (HostedInputDisposition, HostedProgramUpdate) {
        match self.dispatch_input(id, event) {
            Ok(result) => (
                HostedInputDisposition {
                    prevent_default: result.default_prevented || result.consumed,
                },
                self.program_update(true),
            ),
            Err(_) => (
                HostedInputDisposition::default(),
                HostedProgramUpdate::default(),
            ),
        }
    }

    pub fn hosted_window_event(&mut self, event: HostedWindowEvent) -> HostedProgramUpdate {
        let close_primary = matches!(
            event,
            HostedWindowEvent::CloseRequested {
                id: HostedWindowId::PRIMARY,
                ..
            }
        );
        if let Err(_error) = self.handle_window_event(event) {
            return HostedProgramUpdate::default();
        }
        if close_primary {
            return HostedProgramUpdate::exit();
        }
        self.program_update(true)
    }

    pub fn hosted_runtime_event(&mut self, event: HostedRuntimeEvent) -> HostedProgramUpdate {
        if let Err(_error) = self.handle_runtime_event(event) {
            return HostedProgramUpdate::default();
        }
        self.program_update(true)
    }

    pub fn hosted_rebuild_gpu(&mut self, resources: HostedGpuResources) -> HostedProgramUpdate {
        match self
            .vue
            .replace_host_gpu(&mut self.engine, resources, "hosted GPU device recovered")
        {
            Ok(_) => {
                let _ = self.register_complete_host_api();
                self.program_update(true)
            }
            Err(_) => HostedProgramUpdate::default(),
        }
    }

    pub fn hosted_wake(&mut self) -> HostedProgramUpdate {
        match self.pump() {
            Ok(work) if work > 0 => self.program_update(true),
            _ => self.program_update(false),
        }
    }

    fn program_update(&self, redraw: bool) -> HostedProgramUpdate {
        let commands = self.drain_window_commands();
        let update = if redraw {
            HostedProgramUpdate::redraw_all()
        } else {
            HostedProgramUpdate::default()
        };
        update.with_window_commands(commands)
    }

    pub fn handle_window_event(&mut self, event: HostedWindowEvent) -> Result<(), JsEngineError> {
        match event {
            HostedWindowEvent::Ready { id, geometry, .. } => {
                let id = VueWindowId(id.0);
                self.vue.set_viewport(
                    id,
                    geometry.physical_size.width,
                    geometry.physical_size.height,
                    geometry.scale_factor,
                )?;
                self.vue.record_geometry(id, &geometry)?;
                self.vue.bind_window(&mut self.engine, id)?;
                self.vue.notify_window_ready(id)?;
                self.vue.pump_lifecycle(
                    &mut self.engine,
                    id,
                    WindowLifecycleEvent::ResizeWithScale {
                        width: geometry.logical_size.width as f64,
                        height: geometry.logical_size.height as f64,
                        scale_factor: geometry.scale_factor as f64,
                    },
                )?;
            }
            HostedWindowEvent::Resized { id, geometry, .. } => {
                let id = VueWindowId(id.0);
                self.vue.set_viewport(
                    id,
                    geometry.physical_size.width,
                    geometry.physical_size.height,
                    geometry.scale_factor,
                )?;
                self.vue.record_geometry(id, &geometry)?;
                self.vue.pump_lifecycle(
                    &mut self.engine,
                    id,
                    WindowLifecycleEvent::ResizeWithScale {
                        width: geometry.logical_size.width as f64,
                        height: geometry.logical_size.height as f64,
                        scale_factor: geometry.scale_factor as f64,
                    },
                )?;
            }
            HostedWindowEvent::VisibilityChanged { id, hidden, .. } => {
                self.vue.pump_lifecycle(
                    &mut self.engine,
                    VueWindowId(id.0),
                    WindowLifecycleEvent::VisibilityChange { hidden },
                )?;
            }
            HostedWindowEvent::FocusChanged { id, focused, .. } => {
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
            HostedWindowEvent::Ime { id, event, .. } => {
                self.vue
                    .dispatch_native_ime(&mut self.engine, VueWindowId(id.0), &event)?;
            }
            HostedWindowEvent::Closed { id, .. } => {
                self.vue.notify_window_closed(VueWindowId(id.0))?;
            }
            HostedWindowEvent::Moved { id, geometry, .. } => {
                self.vue.record_geometry(VueWindowId(id.0), &geometry)?;
            }
            HostedWindowEvent::CloseRequested { id, .. } => {
                if id != HostedWindowId::PRIMARY {
                    self.vue.request_close(VueWindowId(id.0))?;
                }
            }
            HostedWindowEvent::FileHovered {
                id, path, position, ..
            } => {
                let host = self.require_host(VueWindowId(id.0))?;
                host.lock()
                    .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                    .dispatch_file_drag(
                        &mut self.engine,
                        FileDragEventKind::Hover,
                        std::slice::from_ref(&path),
                        position.map(|point| (point.x, point.y)),
                    )?;
            }
            HostedWindowEvent::FilesHovered {
                id,
                paths,
                position,
                ..
            } => {
                let host = self.require_host(VueWindowId(id.0))?;
                host.lock()
                    .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                    .dispatch_file_drag(
                        &mut self.engine,
                        FileDragEventKind::Hover,
                        &paths,
                        position.map(|point| (point.x, point.y)),
                    )?;
            }
            HostedWindowEvent::FileDropped {
                id, path, position, ..
            } => {
                let host = self.require_host(VueWindowId(id.0))?;
                host.lock()
                    .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                    .dispatch_file_drag(
                        &mut self.engine,
                        FileDragEventKind::Drop,
                        std::slice::from_ref(&path),
                        position.map(|point| (point.x, point.y)),
                    )?;
            }
            HostedWindowEvent::FilesDropped {
                id,
                paths,
                position,
                ..
            } => {
                let host = self.require_host(VueWindowId(id.0))?;
                host.lock()
                    .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                    .dispatch_file_drag(
                        &mut self.engine,
                        FileDragEventKind::Drop,
                        &paths,
                        position.map(|point| (point.x, point.y)),
                    )?;
            }
            HostedWindowEvent::FileHoverCancelled { id, .. } => {
                let host = self.require_host(VueWindowId(id.0))?;
                host.lock()
                    .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                    .dispatch_file_drag(&mut self.engine, FileDragEventKind::Cancel, &[], None)?;
            }
            // Keyboard input already reaches Vue first through `HostedInputEvent`.
            // This later host shortcut notification must not emit a duplicate JS event.
            HostedWindowEvent::KeyPressed { .. } => {}
        }
        Ok(())
    }

    pub fn handle_runtime_event(&mut self, event: HostedRuntimeEvent) -> Result<(), JsEngineError> {
        if let HostedRuntimeEvent::WindowOpenFailed { id, message } = event {
            self.vue
                .notify_window_open_failed(VueWindowId(id.0), message)?;
        }
        Ok(())
    }

    pub fn drain_window_commands(&self) -> Vec<HostedWindowCommand> {
        self.vue.drain_hosted_window_commands()
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

fn map_modifiers(value: nana_ui::HostedInputModifiers) -> InputModifiers {
    InputModifiers {
        alt: value.alt,
        control: value.control,
        meta: value.meta,
        shift: value.shift,
    }
}

/// Ready-to-run hosted program owning one [`VueHostedRuntime`].
pub struct VueHostedProgram<E: JsEngine> {
    runtime: VueHostedRuntime<E>,
    theme: ThemeMode,
    appearance: AppearanceSettings,
}

impl<E: JsEngine> VueHostedProgram<E> {
    pub fn bootstrap(
        context: &HostedProgramContext<BridgeEvent>,
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
    ) -> Result<Self, JsEngineError> {
        let geometry = context.geometry();
        let mut runtime = VueHostedRuntime::new(
            engine,
            artifact,
            application_api,
            geometry.physical_size.width.max(1),
            geometry.physical_size.height.max(1),
            geometry.scale_factor.max(0.01),
        )?;
        runtime.bind_host_gpu(context.gpu().clone())?;
        runtime
            .vue
            .record_geometry(VueWindowId::PRIMARY, &geometry)?;
        Ok(Self {
            runtime,
            theme: ThemeMode::Light,
            appearance: AppearanceSettings::default(),
        })
    }

    pub fn runtime(&self) -> &VueHostedRuntime<E> {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut VueHostedRuntime<E> {
        &mut self.runtime
    }
}

impl<E: JsEngine + 'static> VueHostedProgram<E> {
    /// Production entry for caller-owned engines. Release applications pass a
    /// `nana_js_v8::V8Engine` here, keeping one engine for every Vue window.
    pub fn run(
        settings: nana_ui::HostedWindowSettings,
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
    ) -> Result<(), nana_ui::HostedRunError> {
        nana_ui::run_hosted_with::<Self, _>(settings, move |context| {
            Self::bootstrap(context, engine, artifact, application_api)
                .map(|program| (program, Vec::new()))
        })
    }
}

impl<E: JsEngine + 'static> HostedProgram for VueHostedProgram<E> {
    type Message = BridgeEvent;
    type Error = JsEngineError;

    fn initialize(
        _context: &HostedProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        Err(JsEngineError::new(
            "VueHostedProgram::bootstrap must create the engine and runtime artifact",
        ))
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        match self
            .runtime
            .dispatch_bridge_event(HostedWindowId::PRIMARY, message)
        {
            Ok(_) => self.runtime.program_update(true),
            Err(_) => HostedProgramUpdate::default(),
        }
    }

    fn view(&self, native_material: bool) -> Element<'static, Self::Message> {
        self.view_window(HostedWindowId::PRIMARY, native_material)
    }

    fn view_window(
        &self,
        id: HostedWindowId,
        native_material: bool,
    ) -> Element<'static, Self::Message> {
        self.runtime
            .view_window(id, native_material)
            .unwrap_or_else(|_| iced::widget::Space::new().into())
    }

    fn theme_mode(&self) -> ThemeMode {
        self.theme
    }

    fn accessibility_snapshot(
        &self,
        id: HostedWindowId,
    ) -> Vec<nana_ui_runtime::AccessibilityNode> {
        self.runtime.accessibility_snapshot(id)
    }

    fn accessibility_adapter_enabled(&self) -> bool {
        true
    }

    fn accessibility_update(
        &mut self,
        id: HostedWindowId,
    ) -> Option<nana_ui_runtime::AccessibilityUpdate> {
        self.runtime.take_accessibility_update(id)
    }

    fn accessibility_actions_enabled(&self) -> bool {
        true
    }

    fn accessibility_action(
        &mut self,
        id: HostedWindowId,
        request: nana_ui_runtime::AccessibilityActionRequest,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        match self.runtime.hosted_accessibility_action(id, request) {
            Ok(update) => update,
            Err(_) => HostedProgramUpdate::default(),
        }
    }

    fn window_material_mode(&self) -> WindowMaterialMode {
        self.appearance.window_material()
    }

    fn backdrop_opacity(&self) -> f32 {
        self.appearance.backdrop_opacity()
    }

    fn backdrop_target(&self) -> BackdropTarget {
        self.appearance.backdrop_target()
    }

    fn titlebar_follows_sidebar(&self) -> bool {
        self.appearance.titlebar_follows_sidebar()
    }

    fn window_event(
        &mut self,
        event: HostedWindowEvent,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        self.runtime.hosted_window_event(event)
    }

    fn text_input_request(&self, id: HostedWindowId) -> Option<nana_ui_platform::TextInputRequest> {
        let host = self.runtime.vue.host(VueWindowId(id.0))?;
        let host = host.lock().ok()?;
        host.text_input_request()
    }

    fn input_event(
        &mut self,
        id: HostedWindowId,
        event: HostedInputEvent,
        _context: &HostedProgramContext<Self::Message>,
    ) -> (HostedInputDisposition, HostedProgramUpdate) {
        self.runtime.hosted_input(id, event)
    }

    fn runtime_event(
        &mut self,
        event: HostedRuntimeEvent,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        self.runtime.hosted_runtime_event(event)
    }

    fn next_wakeup(&self) -> Option<Instant> {
        self.runtime.next_wakeup()
    }

    fn wake(
        &mut self,
        _now: Instant,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        self.runtime.hosted_wake()
    }

    fn rebuild_gpu(&mut self, context: &HostedProgramContext<Self::Message>) {
        let _ = self.runtime.hosted_rebuild_gpu(context.gpu().clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
