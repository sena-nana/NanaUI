//! Production controller joining one JS engine, all Vue documents, and the
//! NanaUI Scene/`run_runtime` host.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nana_js_engine::{HostApiRegistry, JsEngine, JsEngineError, RuntimeArtifact};
use nana_ui::{
    GpuContext, HostTextureRegistry, RoutedInput, RuntimeProgram, RuntimeProgramContext,
    RuntimeProgramUpdate, RuntimeRedraw, ThemeAppearance, WindowDescriptor, window_material_effect,
};
use nana_ui_platform::{CanonicalInputEvent, InputPayload, WindowEvent, WindowGeometry, WindowId};
use nana_ui_runtime::FrameworkError;
use nana_ui_scene::RuntimeDocument;

use crate::{
    BridgeEvent, KeyboardInput, PointerInput, SharedRuntimeDocument, VueRuntime, VueWindowId,
    WheelInput, WindowLifecycleEvent, theme_tokens_from_appearance,
};

thread_local! {
    static PENDING_VUE_BOOTSTRAP: RefCell<Option<Box<dyn Any>>> = RefCell::new(None);
}

struct PendingVueBootstrapGuard;

impl Drop for PendingVueBootstrapGuard {
    fn drop(&mut self) {
        PENDING_VUE_BOOTSTRAP.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

fn install_pending_vue_bootstrap(bootstrap: Box<dyn Any>) -> PendingVueBootstrapGuard {
    PENDING_VUE_BOOTSTRAP.with(|slot| {
        *slot.borrow_mut() = Some(bootstrap);
    });
    PendingVueBootstrapGuard
}

/// A single-engine Vue runtime suitable for embedding in `RuntimeProgram`.
pub struct VueHostedRuntime<E: JsEngine> {
    engine: E,
    vue: VueRuntime,
    application_api: HostApiRegistry,
}

#[derive(PartialEq)]
struct WindowVisualRevision {
    semantic: u64,
    runtime: u64,
    surfaces: HashMap<String, SurfaceRevision>,
}

#[derive(PartialEq)]
struct SurfaceRevision {
    canvas: Option<u64>,
    texture: Option<(u64, u64, u64, u32, u32, nana_ui::HostTextureAlphaMode)>,
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
        Self::with_store(
            engine,
            artifact,
            application_api,
            physical_width,
            physical_height,
            scale_factor,
            nana_ui_core::memory_store(),
        )
    }

    pub fn with_store(
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        store: nana_ui_core::SharedStore,
    ) -> Result<Self, JsEngineError> {
        Self::from_vue(
            engine,
            VueRuntime::with_store(physical_width, physical_height, scale_factor, store),
            artifact,
            application_api,
        )
    }

    /// State recorded on `vue` beforehand is visible to the script's first run.
    fn from_vue(
        engine: E,
        vue: VueRuntime,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
    ) -> Result<Self, JsEngineError> {
        let mut runtime = Self {
            engine,
            vue,
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

    pub fn inject_theme(&mut self, theme: ThemeAppearance) -> Result<(), JsEngineError> {
        let host = self.require_host(VueWindowId::PRIMARY)?;
        host.lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .inject_theme(&mut self.engine, theme)
    }

    pub fn inject_stylesheet(&self, css: &str) -> Result<(), JsEngineError> {
        self.vue.inject_stylesheet(css)
    }

    /// Replace the artifact and rebuild the tree, keeping the window alive.
    ///
    /// Takes an engine **factory**, not an engine. V8 enters an isolate when it
    /// is created and exits it when it is dropped, and requires strict LIFO
    /// ordering, so building the replacement before releasing the old one
    /// aborts the process on the first reload. The old engine is shut down
    /// first and the new one is built only afterwards; a caller that hands over
    /// an already-constructed engine cannot express that order.
    ///
    /// `crate::dev` explains why the isolate is replaced at all rather than
    /// re-evaluating into the surviving one.
    ///
    /// Deliberately absent: any call to [`Self::bind_host_gpu`]. The GPU is
    /// bound on `VueHost`, not on the engine, so the `Device`, `Queue`,
    /// `Surface` and every host texture survive untouched. That is the whole
    /// difference between this and restarting the process.
    ///
    /// On an evaluation failure the previous artifact is re-evaluated so the
    /// developer is left with the app they had rather than a blank window, and
    /// the original error is returned.
    #[cfg(feature = "dev-reload")]
    pub fn dev_reload(
        &mut self,
        artifact: &RuntimeArtifact,
        make_engine: &(dyn Fn() -> E + Send + Sync),
        previous: Option<&RuntimeArtifact>,
        geometry: Option<&nana_ui_platform::WindowGeometry>,
        theme: ThemeAppearance,
    ) -> Result<Vec<nana_ui_platform::host::WindowCommand>, JsEngineError> {
        // The artifact mounts into the primary document. Without it a reload has
        // nowhere to mount, so leave the surviving windows and their content alone.
        self.require_host(VueWindowId::PRIMARY).map_err(|_| {
            JsEngineError::new("dev reload requires the primary Vue window, which is closed")
        })?;
        let state = crate::dev::save_state(&mut self.engine);

        // One isolate is shared by every window, so auxiliary windows cannot
        // outlive the reload. The reloaded artifact opens them again.
        let window_commands = self.vue.dev_close_auxiliary_windows();

        self.vue.dev_teardown()?;

        // Throw the JS heap away *before* building the replacement: V8 enters
        // an isolate on creation and exits it on drop, strictly LIFO, so the
        // other order aborts the process. `shutdown` releases the isolate in
        // place, leaving the shell replaced below owning nothing.
        self.engine.shutdown();
        self.engine = make_engine();

        // Re-registers the complete host API -- including the GPU-bound ops,
        // because the registry is rebuilt from current host state -- evaluates
        // the artifact, and rebinds the event bridge for every live window.
        crate::dev::publish_restore_state(&mut self.engine, state.as_deref())?;
        if let Err(error) =
            self.vue
                .initialize(&mut self.engine, artifact.clone(), &self.application_api)
        {
            self.dev_restore_previous(previous, state.as_deref());
            return Err(error);
        }

        // What the new context cannot know on its own.
        if let Some(geometry) = geometry {
            self.vue
                .record_platform_geometry(VueWindowId::PRIMARY, geometry)?;
        }
        self.inject_theme(theme)?;

        let mut commands = window_commands;
        commands.extend(self.vue.drain_runtime_window_commands());
        Ok(commands)
    }

    /// Put the last known-good artifact back after a failed reload.
    ///
    /// Best effort by construction: the caller already has an error to report,
    /// and a failure here would only replace it with a less useful one.
    #[cfg(feature = "dev-reload")]
    fn dev_restore_previous(&mut self, previous: Option<&RuntimeArtifact>, state: Option<&str>) {
        let Some(previous) = previous else {
            return;
        };
        let _ = self.vue.dev_teardown();
        let _ = crate::dev::publish_restore_state(&mut self.engine, state);
        let _ = self
            .vue
            .initialize(&mut self.engine, previous.clone(), &self.application_api);
    }

    pub fn bind_host_gpu(&mut self, resources: GpuContext) -> Result<u64, JsEngineError> {
        let generation = self.vue.bind_host_gpu(resources)?;
        self.register_complete_host_api()?;
        Ok(generation)
    }

    /// Route an event to the window that owns its widget.
    pub fn dispatch_bridge_event(&mut self, event: BridgeEvent) -> Result<bool, JsEngineError> {
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

    /// Route one event into a Vue window's document through its own input
    /// source, then let the page observe it. For a host with no native
    /// window: tests, agents, offscreen harnesses. A key press and the text
    /// it types are two events.
    pub fn runtime_input(
        &mut self,
        id: WindowId,
        payload: InputPayload,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let before = self.window_visual_revisions();
        let window = VueWindowId(id.0);
        // A callback may update another document or mutate state before
        // throwing; those changes stand even when delivery reports an error.
        if let Ok((event, outcome)) = self.route_standalone(window, payload) {
            let _ = self.observe_runtime_canonical(window, &event, outcome.disposition());
        }
        Ok(self.update_for_changed_windows(before))
    }

    /// Observe input the native scene host has already routed to UiWorld.
    fn observe_routed_input(
        &mut self,
        id: WindowId,
        input: RoutedInput<'_>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let before = self.window_visual_revisions();
        let _ = self.observe_runtime_canonical(VueWindowId(id.0), input.event, input.disposition);
        Ok(self.update_for_changed_windows(before))
    }

    /// Stamp and route `payload` for a window with no native host. The time
    /// is the animation clock's, so tooltip delays and click timing run as
    /// they do in a window.
    fn route_standalone(
        &mut self,
        window: VueWindowId,
        payload: InputPayload,
    ) -> Result<(CanonicalInputEvent, nana_ui::InputRouteOutcome), JsEngineError> {
        self.require_host(window)?
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
            .route_input(payload)
    }

    /// Emit the browser-shaped events a page sees for one routed event. This
    /// is observation only: hit testing, capture and focus already happened
    /// in the one Runtime route.
    fn observe_runtime_canonical(
        &mut self,
        window: VueWindowId,
        event: &CanonicalInputEvent,
        disposition: nana_ui::InputDisposition,
    ) -> Result<(), JsEngineError> {
        let host = self.require_host(window)?;
        let mut host = host
            .lock()
            .map_err(|_| JsEngineError::new("Vue window host poisoned"))?;
        let engine = &mut self.engine;
        match &event.payload {
            InputPayload::Pointer(pointer) => {
                host.emit_pointer_from_runtime(engine, PointerInput::from_canonical(pointer))?;
            }
            InputPayload::Wheel(wheel) => {
                host.emit_wheel_from_runtime(engine, WheelInput::from_canonical(wheel))?;
            }
            InputPayload::Key(key) => {
                let allowed = host.emit_keyboard_from_runtime(
                    engine,
                    &KeyboardInput::from_canonical(key),
                    None,
                )?;
                // Text a handled or prevented key typed is not typed: the
                // page sees no `input` for it, as a browser would not.
                if key.is_pressed() && (disposition.handled || !allowed) {
                    host.input_projection.handled_key = Some(event.metadata.sequence);
                }
            }
            InputPayload::Text(committed) => {
                let suppressed =
                    committed.key.is_some() && host.input_projection.handled_key == committed.key;
                let target = host.focused_text_input();
                if let Some(target) = target.filter(|_| !suppressed && !committed.text.is_empty()) {
                    host.emit_text_events_from_runtime(
                        engine,
                        target,
                        &committed.text,
                        "insertText",
                    )?;
                }
            }
            InputPayload::Composition(composition) => {
                // A blocking Runtime overlay owns the composition; the page
                // does not see it.
                let blocked = host.document().lock().is_ok_and(|document| {
                    let runtime = document.runtime_document();
                    runtime
                        .context()
                        .has_blocking_runtime_overlay(runtime.document())
                });
                if !blocked {
                    host.emit_native_ime_from_runtime(engine, composition, disposition.handled)?;
                }
            }
            InputPayload::FileDrag(drag) => {
                host.emit_file_drag_from_runtime(engine, drag.kind, &drag.paths, drag.position)?;
            }
            InputPayload::PointerEnter { .. }
            | InputPayload::PointerLeave { .. }
            | InputPayload::Focus { .. }
            | InputPayload::DeviceConnected
            | InputPayload::DeviceDisconnected
            | InputPayload::SourceConnected
            | InputPayload::SourceDisconnected => {}
        }
        Ok(())
    }

    /// Redraw only the windows whose visual revision moved since `before`.
    fn update_for_changed_windows(
        &self,
        before: HashMap<WindowId, WindowVisualRevision>,
    ) -> RuntimeProgramUpdate {
        let after = self.window_visual_revisions();
        let redraw = RuntimeRedraw::for_windows(
            after
                .into_iter()
                .filter_map(|(id, revision)| (before.get(&id) != Some(&revision)).then_some(id)),
        );
        RuntimeProgramUpdate {
            redraw,
            ..self.runtime_program_update(false)
        }
    }

    fn window_visual_revisions(&self) -> HashMap<WindowId, WindowVisualRevision> {
        self.vue
            .window_ids()
            .into_iter()
            .filter_map(|id| {
                let host = self.vue.host(id)?;
                let host = host.lock().ok()?;
                let semantic = host.bridge().lock().ok()?.revision();
                let document = host.document();
                let document = document.lock().ok()?;
                let canvas = host.canvas_runtime_ref().clone();
                let canvas = canvas.lock().ok()?;
                let textures = host.host_textures();
                let surfaces = document
                    .consumed_surface_slots()
                    .map(|slot| {
                        let canvas = slot
                            .strip_prefix("canvas:")
                            .and_then(|id| id.parse().ok())
                            .and_then(|id| canvas.version(nana_ui_web_api::CanvasId(id)));
                        let texture = textures.get(&slot).map(|binding| {
                            (
                                binding.texture.instance_identity(),
                                binding.texture.generation(),
                                binding.texture.version(),
                                binding.width,
                                binding.height,
                                binding.alpha_mode,
                            )
                        });
                        (slot, SurfaceRevision { canvas, texture })
                    })
                    .collect();
                Some((
                    WindowId(id.0),
                    WindowVisualRevision {
                        semantic,
                        runtime: document.runtime_generation(),
                        surfaces,
                    },
                ))
            })
            .collect()
    }

    /// The host already redraws a window it resizes or publishes; moves, focus
    /// and occlusion repaint only windows whose Vue state actually changed.
    pub fn runtime_window_event(&mut self, event: WindowEvent) -> RuntimeProgramUpdate {
        self.runtime_window_event_delivery(event, false)
    }

    fn runtime_window_event_delivery(
        &mut self,
        event: WindowEvent,
        already_routed: bool,
    ) -> RuntimeProgramUpdate {
        // These run no script, so no Vue window can change visually.
        let scripted = !matches!(
            event,
            WindowEvent::Moved { .. }
                | WindowEvent::MousePassthroughChanged { .. }
                | WindowEvent::SkipTaskbarChanged { .. }
                | WindowEvent::AppearanceChanged { .. }
                | WindowEvent::FileDialogRejected { .. }
                | WindowEvent::FileDialogCompleted { .. }
        );
        let before = scripted.then(|| self.window_visual_revisions());
        if let Err(_error) = self.apply_window_event(event, already_routed) {
            return RuntimeProgramUpdate::default();
        }
        match before {
            Some(before) => self.update_for_changed_windows(before),
            None => self.runtime_program_update(false),
        }
    }

    /// Complete viewport and JS binding before the host publishes the window.
    pub fn prepare_window_creation(
        &mut self,
        id: WindowId,
        geometry: nana_ui_platform::WindowGeometry,
    ) -> Result<(), JsEngineError> {
        let id = VueWindowId(id.0);
        self.vue.set_viewport(
            id,
            geometry.physical_size.0.max(1),
            geometry.physical_size.1.max(1),
            geometry.scale_factor.max(0.01),
        )?;
        // An isolated window's script runs here, against the real viewport and
        // before the host publishes the window.
        self.vue.bind_window(&mut self.engine, id)?;
        self.vue.record_platform_geometry(id, &geometry)?;
        Ok(())
    }

    fn handle_platform_window_event(&mut self, event: WindowEvent) -> Result<(), JsEngineError> {
        self.apply_window_event(event, false)
    }

    fn apply_window_event(
        &mut self,
        event: WindowEvent,
        already_routed: bool,
    ) -> Result<(), JsEngineError> {
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
                let window = VueWindowId(id.0);
                if !already_routed {
                    self.route_standalone(window, InputPayload::Focus { focused })?;
                }
                self.vue.pump_lifecycle(
                    &mut self.engine,
                    window,
                    if focused {
                        WindowLifecycleEvent::Focus
                    } else {
                        WindowLifecycleEvent::Blur
                    },
                )?;
            }
            WindowEvent::OpenFailed { id, .. } => {
                let window = VueWindowId(id.0);
                self.detach_input_source(window)?;
                let closed = self.vue.notify_window_closed(window);
                self.vue.dispose_released_realms(&mut self.engine)?;
                closed?;
            }
            // Pages observe the pointer through their own DOM pointer events;
            // window presence is for Rust programs with hover-revealed chrome.
            WindowEvent::MousePassthroughChanged { .. }
            | WindowEvent::SkipTaskbarChanged { .. }
            | WindowEvent::PointerPresenceChanged { .. }
            | WindowEvent::ReducedMotionChanged { .. } => {}
            WindowEvent::ModeChanged { id, mode } => {
                self.vue.record_platform_mode(VueWindowId(id.0), &mode)?;
            }
            // File dialogs are owned by the application, not the framework:
            // `PathField` emits `BrowseRequested`, the application answers with
            // `WindowCommand::OpenFileDialog` and consumes the outcome. The Vue
            // host never sends that command, so it has no request to settle
            // here and no JS channel to deliver one to.
            WindowEvent::FileDialogRejected { .. } | WindowEvent::FileDialogCompleted { .. } => {}
            // The Vue host has no appearance channel: theme reaches JS through
            // the application's own state, not a lifecycle event. Rust programs
            // read it from `RuntimeProgram::window_event`.
            WindowEvent::AppearanceChanged { .. } => {}
            WindowEvent::Closed { id } => {
                let window = VueWindowId(id.0);
                self.detach_input_source(window)?;
                self.vue.notify_window_closed(window)?;
                // The last closed document cannot supply another frame pump.
                // Deliver its reliable lifecycle event while the engine is alive.
                let delivered = self.engine.run_microtasks();
                // An isolated realm whose last window this was is released even
                // when one of its listeners threw during that delivery.
                self.vue.dispose_released_realms(&mut self.engine)?;
                delivered?;
            }
            WindowEvent::Moved { id, geometry } => {
                self.vue
                    .record_platform_geometry(VueWindowId(id.0), &geometry)?;
            }
            WindowEvent::CloseRequested { id } => {
                self.vue.request_close(VueWindowId(id.0))?;
            }
        }
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

    pub fn runtime_rebuild_gpu(&mut self, resources: GpuContext) -> RuntimeProgramUpdate {
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

    pub fn apply_css_animation_frame(
        &mut self,
        id: WindowId,
        frame: nana_ui_runtime::AnimationFrame,
    ) -> bool {
        let Ok(host) = self.require_host(VueWindowId(id.0)) else {
            return false;
        };
        let Ok(host) = host.lock() else {
            return false;
        };
        let bridge_slot = host.bridge();
        let Ok(mut bridge) = bridge_slot.lock() else {
            return false;
        };
        let document_slot = host.document();
        let Ok(mut doc) = document_slot.lock() else {
            return false;
        };
        let changed = bridge.apply_css_animation_samples(&mut doc, frame);
        drop(doc);
        drop(bridge);
        drop(host);
        // Same beat as sample apply so T_end does not wait for the next pump
        // (where a class-arm fallback timeout could fire first).
        if let Ok(host) = self.require_host(VueWindowId(id.0))
            && let Ok(host) = host.lock()
        {
            let _ = host.flush_motion_complete(&mut self.engine);
        }
        changed
    }

    pub fn sync_animation_clock(&mut self, epoch: std::time::Instant) {
        self.vue.sync_animation_clock(epoch);
    }

    pub fn prepare_runtime_window(&self, id: WindowId) {
        let Some(host) = self.vue.host(VueWindowId(id.0)) else {
            return;
        };
        let Ok(mut host) = host.lock() else {
            return;
        };
        host.prepare_window_frame();
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

    /// Forget a closing window's input source; what it held is cancelled
    /// while its document is alive, and a reused id starts a newer
    /// generation.
    fn detach_input_source(&mut self, window: VueWindowId) -> Result<(), JsEngineError> {
        match self.vue.host(window) {
            Some(host) => host
                .lock()
                .map_err(|_| JsEngineError::new("Vue window host poisoned"))?
                .detach_input_source(),
            None => Ok(()),
        }
    }

    fn register_complete_host_api(&mut self) -> Result<(), JsEngineError> {
        self.vue.register_host_apis(&mut self.engine)
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
    theme: ThemeAppearance,
    compiled_theme: Arc<nana_ui::CompiledTheme>,
    /// `Nana.startup`, when this program was bootstrapped by the host.
    startup: Option<crate::startup::StartupBridge>,
    #[cfg(feature = "dev-reload")]
    dev: Option<DevState<E>>,
}

/// Host-level message for [`VueRuntimeProgram`].
///
/// `RuntimeProgram::Message` is documented as the channel for host-level work,
/// not for widget input, so the two are kept apart here: `Input` carries a user
/// action to the Vue tree, `Dev` carries a command addressed to the host. Before
/// this split a dev-only reload had to travel as a [`BridgeEvent`] variant,
/// which forced every widget-event match in the crate to answer questions —
/// "which widget?", "which JS event name?" — that a reload has no answer for.
#[derive(Debug, Clone, PartialEq)]
pub enum VueMessage {
    Input(BridgeEvent),
    #[cfg(feature = "dev-reload")]
    Dev(crate::dev::DevReload),
}

impl From<BridgeEvent> for VueMessage {
    fn from(event: BridgeEvent) -> Self {
        Self::Input(event)
    }
}

#[cfg(feature = "dev-reload")]
impl From<crate::dev::DevReload> for VueMessage {
    fn from(request: crate::dev::DevReload) -> Self {
        Self::Dev(request)
    }
}

/// Everything [`RuntimeProgram::initialize`] needs, handed across the
/// thread-local slot that `run_runtime` opens between the caller and the
/// program it constructs.
struct VueBootstrap<E: JsEngine> {
    engine: E,
    artifact: RuntimeArtifact,
    application_api: HostApiRegistry,
    #[cfg(feature = "dev-reload")]
    dev: Option<DevState<E>>,
}

/// Everything a reload needs that the running program does not otherwise keep.
#[cfg(feature = "dev-reload")]
struct DevState<E: JsEngine> {
    /// Builds the fresh isolate each reload runs in. A factory rather than an
    /// engine because `JsEngine` has no constructor in its contract.
    engine: std::sync::Arc<dyn Fn() -> E + Send + Sync>,
    /// The last artifact that evaluated successfully. A save with a syntax
    /// error is put back to this rather than leaving a blank window.
    last_good: Option<std::sync::Arc<RuntimeArtifact>>,
}

/// Historical name for [`VueRuntimeProgram`].
pub type VueHostedProgram<E> = VueRuntimeProgram<E>;

impl<E: JsEngine> VueRuntimeProgram<E> {
    pub fn bootstrap(
        context: &RuntimeProgramContext<VueMessage>,
        engine: E,
        artifact: RuntimeArtifact,
        mut application_api: HostApiRegistry,
    ) -> Result<Self, JsEngineError> {
        let geometry = context.geometry();
        let startup = crate::startup::StartupBridge::new(context.startup().clone());
        // A clash with the application's own names fails startup, like any
        // other framework host API.
        let mut startup_api = HostApiRegistry::new();
        startup.register(&mut startup_api);
        application_api.try_extend(&startup_api)?;
        let mut program = Self::bootstrap_from_gpu(
            context.gpu().clone(),
            geometry.physical_size.0.max(1),
            geometry.physical_size.1.max(1),
            geometry.scale_factor.max(0.01),
            Some(geometry),
            context.window_tag(),
            engine,
            artifact,
            application_api,
            Arc::clone(context.store()),
        )?;
        program.startup = Some(startup);
        Ok(program)
    }

    fn bootstrap_from_gpu(
        gpu: GpuContext,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        platform_geometry: Option<WindowGeometry>,
        primary_tag: Option<&str>,
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
        store: nana_ui_core::SharedStore,
    ) -> Result<Self, JsEngineError> {
        let vue = VueRuntime::with_store(physical_width, physical_height, scale_factor, store);
        vue.set_window_tag(VueWindowId::PRIMARY, primary_tag.map(str::to_owned))?;
        let mut runtime = VueHostedRuntime::from_vue(engine, vue, artifact, application_api)?;
        runtime.bind_host_gpu(gpu)?;
        if let Some(geometry) = platform_geometry {
            runtime
                .vue
                .record_platform_geometry(VueWindowId::PRIMARY, &geometry)?;
        }
        let _ = runtime.inject_theme(ThemeAppearance::Light);
        let mut program = Self {
            runtime,
            documents: HashMap::new(),
            theme: ThemeAppearance::Light,
            compiled_theme: nana_ui::builtin_theme_arc(ThemeAppearance::Light),
            startup: None,
            #[cfg(feature = "dev-reload")]
            dev: None,
        };
        program.sync_documents();
        Ok(program)
    }

    pub fn from_runtime(runtime: VueHostedRuntime<E>) -> Self {
        let mut program = Self {
            runtime,
            documents: HashMap::new(),
            theme: ThemeAppearance::Light,
            compiled_theme: nana_ui::builtin_theme_arc(ThemeAppearance::Light),
            startup: None,
            #[cfg(feature = "dev-reload")]
            dev: None,
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

#[cfg(feature = "dev-reload")]
impl<E: JsEngine + 'static> VueRuntimeProgram<E> {
    /// Development entry: same as [`Self::run`], but able to reload.
    ///
    /// Takes an engine **factory** rather than an engine. Each reload runs in a
    /// fresh isolate, and `JsEngine` has no constructor in its contract, so the
    /// caller has to supply the one thing only it knows how to build. For a V8
    /// application that is `V8Engine::new`.
    ///
    /// The caller drives reloads by dispatching [`VueMessage::Dev`] through
    /// [`nana_ui::RuntimeProgramContext::dispatch`] -- which is safe from a
    /// watcher thread -- with the file already read. Nothing here touches the
    /// filesystem.
    pub fn run_dev(
        settings: WindowDescriptor,
        engine: impl Fn() -> E + Send + Sync + 'static,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
    ) -> Result<(), nana_ui::HostedRunError> {
        let factory: std::sync::Arc<dyn Fn() -> E + Send + Sync> = std::sync::Arc::new(engine);
        let first = factory();
        let _clear_if_unused = install_pending_vue_bootstrap(Box::new(VueBootstrap {
            engine: first,
            artifact: artifact.clone(),
            application_api,
            dev: Some(DevState {
                engine: factory,
                last_good: Some(std::sync::Arc::new(artifact)),
            }),
        }));
        nana_ui::run_runtime::<Self>(settings)
    }

    /// Apply one reload request. Returns `None` when reloading is not enabled.
    ///
    /// Runs only from [`RuntimeProgram::update`]: the engine must not be on the
    /// stack when it is replaced, which rules out doing this from a host op or
    /// part-way through a frame.
    fn apply_dev_reload(
        &mut self,
        request: crate::dev::DevReload,
        context: &RuntimeProgramContext<VueMessage>,
    ) -> RuntimeProgramUpdate {
        let Some(dev) = self.dev.as_ref() else {
            return RuntimeProgramUpdate::default();
        };
        match request {
            crate::dev::DevReload::Stylesheet { key, css } => {
                // No teardown: the tree, and everything keyed by node id, stays.
                if self.runtime.vue.replace_stylesheet(&key, &css).is_err() {
                    return RuntimeProgramUpdate::default();
                }
                self.runtime.runtime_program_update(true)
            }
            crate::dev::DevReload::Artifact { name, source } => {
                let make_engine = std::sync::Arc::clone(&dev.engine);
                let previous = dev.last_good.clone();

                let artifact = RuntimeArtifact::from_source(name, source);
                let theme = self.theme;
                // The window never moved, so its live geometry is the frame's.
                let geometry = context.geometry();
                let artifact = std::sync::Arc::new(artifact);
                let outcome = self.runtime.dev_reload(
                    &artifact,
                    make_engine.as_ref(),
                    previous.as_deref(),
                    Some(&geometry),
                    theme,
                );
                self.documents.clear();
                self.sync_documents();
                match outcome {
                    Ok(window_commands) => {
                        if let Some(dev) = self.dev.as_mut() {
                            dev.last_good = Some(artifact);
                        }
                        RuntimeProgramUpdate {
                            redraw: RuntimeRedraw::All,
                            window_commands,
                            exit: false,
                        }
                    }
                    Err(error) => {
                        // The previous artifact is already back on screen; report
                        // the failure through the diagnostics channel so the
                        // developer sees why the save did not take.
                        self.runtime.vue.report_dev_reload_failure(&error);
                        self.runtime.runtime_program_update(true)
                    }
                }
            }
        }
    }
}

impl<E: JsEngine + 'static> VueRuntimeProgram<E> {
    /// Production entry for caller-owned engines. Release applications pass a
    /// `nana_js_v8::V8Engine` here, keeping one engine for every Vue window.
    pub fn run(
        settings: WindowDescriptor,
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
    ) -> Result<(), nana_ui::HostedRunError> {
        let _clear_if_unused = install_pending_vue_bootstrap(Box::new(VueBootstrap {
            engine,
            artifact,
            application_api,
            #[cfg(feature = "dev-reload")]
            dev: None,
        }));
        nana_ui::run_runtime::<Self>(settings)
    }

    /// Same as [`Self::run`], with a host-injected persistent store for
    /// application `localStorage`/`Nana.storage`; framework window geometry uses ViewStateStore.
    pub fn run_with_store(
        settings: WindowDescriptor,
        engine: E,
        artifact: RuntimeArtifact,
        application_api: HostApiRegistry,
        store: nana_ui_core::SharedStore,
    ) -> Result<(), nana_ui::HostedRunError> {
        let _clear_if_unused = install_pending_vue_bootstrap(Box::new(VueBootstrap {
            engine,
            artifact,
            application_api,
            #[cfg(feature = "dev-reload")]
            dev: None,
        }));
        nana_ui::run_runtime_with_store::<Self>(settings, store)
    }
}

impl<E: JsEngine + 'static> RuntimeProgram for VueRuntimeProgram<E> {
    type Message = VueMessage;
    type Error = JsEngineError;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let bootstrap = PENDING_VUE_BOOTSTRAP
            .with(|slot| slot.borrow_mut().take())
            .and_then(|boxed| boxed.downcast::<VueBootstrap<E>>().ok())
            .map(|boxed| *boxed)
            .ok_or_else(|| {
                JsEngineError::new(
                    "VueRuntimeProgram::run must supply the engine and runtime artifact",
                )
            })?;
        #[cfg(feature = "dev-reload")]
        let dev = bootstrap.dev;
        let program = Self::bootstrap(
            context,
            bootstrap.engine,
            bootstrap.artifact,
            bootstrap.application_api,
        )?;
        #[cfg(feature = "dev-reload")]
        let program = {
            let mut program = program;
            program.dev = dev;
            program
        };
        Ok((program, Vec::new()))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui_scene::DocumentAccessError> {
        self.documents
            .get(&id)
            .map(|document| document.with_document(f))
            .transpose()
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui_scene::DocumentAccessError> {
        self.sync_documents();
        self.documents
            .get(&id)
            .map(|document| document.with_document_mut(f))
            .transpose()
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        // Single-variant without `dev-reload`, so the match is infallible there.
        #[cfg_attr(
            not(feature = "dev-reload"),
            allow(clippy::infallible_destructuring_match)
        )]
        let event = match message {
            VueMessage::Input(event) => event,
            // A reload replaces the engine, which is only safe with no JS on the
            // stack -- i.e. exactly here, and never mid-frame or inside a host op.
            #[cfg(feature = "dev-reload")]
            VueMessage::Dev(request) => return self.apply_dev_reload(request, _context),
        };
        self.sync_documents();
        match self.runtime.dispatch_bridge_event(event) {
            Ok(_) => {
                self.sync_documents();
                self.runtime.runtime_program_update(true)
            }
            Err(_) => RuntimeProgramUpdate::default(),
        }
    }

    fn theme(&self) -> std::sync::Arc<nana_ui::CompiledTheme> {
        self.compiled_theme.clone()
    }

    fn window_material_mode(&self) -> nana_ui::MaterialEffect {
        self.window_material_mode_for(WindowId::PRIMARY)
    }

    fn window_material_mode_for(&self, id: WindowId) -> nana_ui::MaterialEffect {
        self.runtime
            .vue()
            .host(VueWindowId(id.0))
            .and_then(|host| {
                host.lock()
                    .ok()
                    .map(|guard| window_material_effect(guard.appearance().window_material()))
            })
            .unwrap_or(nana_ui::MaterialEffect::Solid)
    }

    fn appearance_backdrop_opacity(&self) -> f32 {
        self.appearance_backdrop_opacity_for(WindowId::PRIMARY)
    }

    fn appearance_backdrop_opacity_for(&self, id: WindowId) -> f32 {
        self.runtime
            .vue()
            .host(VueWindowId(id.0))
            .and_then(|host| {
                host.lock()
                    .ok()
                    .map(|guard| guard.appearance().backdrop_opacity())
            })
            .unwrap_or(nana_ui::AppearanceSettings::DEFAULT_BACKDROP_OPACITY)
    }

    fn host_textures(&self, id: WindowId) -> Option<HostTextureRegistry> {
        self.runtime.host_textures_for(id)
    }

    /// Each window document's own `fetch()` host also gates its images.
    fn resource_fetch_host(&self, id: WindowId) -> Option<nana_ui::SharedFetchHost> {
        let host = self.runtime.vue().host(VueWindowId(id.0))?;
        host.lock().ok()?.fetch_host()
    }

    fn prepare_window_frame(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        self.sync_documents();
        let appearance = self
            .runtime
            .vue()
            .host(VueWindowId(id.0))
            .and_then(|host| host.lock().ok().map(|guard| guard.appearance()));
        let transparent_surface = context.material().wants_transparent_surface();
        let theme = self.theme();
        if let (Some(appearance), Some(document)) = (appearance, self.documents.get(&id)) {
            let tokens = nana_ui::ThemeTokens::new(theme.style_model().palette, theme.metrics())
                .with_workspace_corners(appearance.workspace_corners_enabled())
                .with_backdrop(
                    transparent_surface,
                    appearance.backdrop_target(),
                    appearance.backdrop_opacity(),
                    appearance.titlebar_follows_sidebar(),
                );
            let mut style_model = theme.style_model();
            style_model.metrics = tokens.metrics;
            style_model.palette = tokens.palette;
            style_model.titlebar = tokens.titlebar;
            let theme = Arc::new(theme.as_ref().clone().with_style_model(style_model));
            if let Err(error) = document
                .with_document_mut(|document| document.context_mut().set_theme_tokens(theme))
            {
                let failure = nana_ui::HostFailure::DocumentAccess {
                    window: id,
                    error: error.to_string(),
                };
                nana_ui::ReportHostFailure::report_host_failure(self, failure);
                return;
            }
        }
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
        input: RoutedInput<'_>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.sync_documents();
        let update = self.runtime.observe_routed_input(id, input)?;
        self.sync_documents();
        Ok(update)
    }

    fn initialize_window(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), String> {
        self.runtime
            .prepare_window_creation(id, context.geometry())
            .map_err(|error| error.to_string())?;
        self.sync_documents();
        self.with_document(id, |_| ())
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Vue window document was not initialized".to_string())
    }

    fn discard_window(&mut self, id: WindowId) {
        let _ = self
            .runtime
            .handle_platform_window_event(WindowEvent::OpenFailed {
                id,
                error: "window initialization failed".into(),
            });
        self.sync_documents();
    }

    fn startup_takeover(&self) -> nana_ui::StartupTakeover {
        if self
            .startup
            .as_ref()
            .is_some_and(crate::startup::StartupBridge::deferred)
        {
            nana_ui::StartupTakeover::Deferred
        } else {
            nana_ui::StartupTakeover::Immediate
        }
    }

    fn startup_changed(
        &mut self,
        status: &nana_ui::StartupStatus,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        let _ = self
            .runtime
            .vue
            .notify_startup(crate::startup::status_value(status));
        RuntimeProgramUpdate::default()
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.sync_documents();
        let update = self.runtime.runtime_window_event_delivery(event, true);
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

    fn sync_animation_clock(&mut self, epoch: Instant) {
        self.runtime.sync_animation_clock(epoch);
    }

    fn animation_frame(
        &mut self,
        id: WindowId,
        frame: nana_ui_runtime::AnimationFrame,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.sync_documents();
        let changed = self.runtime.apply_css_animation_frame(id, frame);
        self.sync_documents();
        Ok(if changed {
            RuntimeProgramUpdate::redraw(id)
        } else {
            RuntimeProgramUpdate::default()
        })
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
    use nana_ui::{
        TitleBarDragTracker, WindowChromeAction, WindowChromeState, apply_title_bar_pointer,
    };
    use nana_ui_platform::{
        DeviceId, EndpointGeneration, InputSequence, InputSourceId, PointerPhase,
    };

    #[test]
    fn pending_vue_bootstrap_guard_clears_untaken_slot() {
        super::PENDING_VUE_BOOTSTRAP.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(()));
        });
        {
            let _guard = super::PendingVueBootstrapGuard;
        }
        super::PENDING_VUE_BOOTSTRAP.with(|slot| {
            assert!(slot.borrow().is_none());
        });
    }

    fn test_runtime() -> VueHostedRuntime<InputEngine> {
        VueHostedRuntime {
            engine: InputEngine::default(),
            vue: VueRuntime::new(400, 300, 1.0),
            application_api: HostApiRegistry::new(),
        }
    }

    fn routed_count(runtime: &VueHostedRuntime<InputEngine>) -> u64 {
        let host = runtime.vue.host(VueWindowId::PRIMARY).unwrap();
        let document = host.lock().unwrap().document();
        let document = document.lock().unwrap();
        document
            .runtime_document()
            .context()
            .input_counters()
            .events_routed
    }

    #[test]
    fn a_reopened_window_routes_at_a_newer_generation() {
        let mut runtime = test_runtime();
        runtime
            .runtime_input(WindowId::PRIMARY, InputPayload::Focus { focused: true })
            .unwrap();
        let generation = |runtime: &VueHostedRuntime<InputEngine>| {
            let host = runtime.vue.host(VueWindowId::PRIMARY).unwrap();
            let document = host.lock().unwrap().document();
            let document = document.lock().unwrap();
            document
                .runtime_document()
                .context()
                .input_binding(nana_ui::HeadlessInput::SOURCE)
                .map(|(generation, _)| generation)
        };
        assert_eq!(generation(&runtime), Some(EndpointGeneration(1)));
        runtime.detach_input_source(VueWindowId::PRIMARY).unwrap();
        assert_eq!(generation(&runtime), None);
        runtime
            .runtime_input(WindowId::PRIMARY, InputPayload::Focus { focused: true })
            .unwrap();
        assert_eq!(generation(&runtime), Some(EndpointGeneration(2)));
    }

    #[test]
    fn native_observation_does_not_apply_text_or_ime_twice() {
        let mut runtime = test_runtime();
        let host = runtime.vue.host(VueWindowId::PRIMARY).unwrap();
        host.lock()
            .unwrap()
            .bind_event_bridge(&mut runtime.engine)
            .unwrap();
        let document = host.lock().unwrap().document();
        let node = {
            let mut document = document.lock().unwrap();
            let retained = document.runtime_document_mut();
            let id = retained.document();
            let node = retained
                .context_mut()
                .create_component(id, nana_ui_runtime::TextInput::new(""))
                .unwrap()
                .stable_id();
            retained.context_mut().focus_node(id, node).unwrap();
            node
        };
        let text = |document: &std::sync::Arc<std::sync::Mutex<crate::NanaTreeDocument>>| {
            document
                .lock()
                .unwrap()
                .runtime_document()
                .context()
                .world()
                .text(node)
                .map(str::to_owned)
        };
        runtime
            .runtime_input(
                WindowId::PRIMARY,
                InputPayload::Text(nana_ui_platform::CommittedText::new("a")),
            )
            .unwrap();
        assert_eq!(text(&document).as_deref(), Some("a"));

        // The scene host routed these already; observing them emits page
        // events and routes nothing.
        let routed = routed_count(&runtime);
        for payload in [
            InputPayload::Text(nana_ui_platform::CommittedText::new("a")),
            InputPayload::Composition(nana_ui_platform::CompositionInput::Commit("文".into())),
        ] {
            let event = CanonicalInputEvent {
                metadata: nana_ui_platform::InputMetadata {
                    source: InputSourceId(1),
                    device: DeviceId(0),
                    generation: EndpointGeneration(1),
                    sequence: InputSequence(1),
                    timestamp: nana_ui_platform::InputTimestamp(0),
                },
                payload,
            };
            runtime
                .observe_routed_input(
                    WindowId::PRIMARY,
                    RoutedInput {
                        event: &event,
                        pointer_hit: None,
                        disposition: Default::default(),
                    },
                )
                .unwrap();
        }
        assert_eq!(routed_count(&runtime), routed);
        assert_eq!(text(&document).as_deref(), Some("a"));
    }

    #[derive(Default)]
    struct InputEngine {
        on_event: Option<Box<dyn FnOnce()>>,
    }

    impl JsEngine for InputEngine {
        fn initialize(&mut self, _: RuntimeArtifact) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn register_host_api(&mut self, _: &HostApiRegistry) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn resolve_function(
            &mut self,
            _: &str,
        ) -> Result<nana_js_engine::JsFunctionId, JsEngineError> {
            Ok(nana_js_engine::JsFunctionId(1))
        }
        fn invoke(
            &mut self,
            _: nana_js_engine::JsFunctionId,
            _: &[nana_js_engine::HostValue],
        ) -> Result<nana_js_engine::HostValue, JsEngineError> {
            if let Some(callback) = self.on_event.take() {
                callback();
            }
            Ok(nana_js_engine::HostValue::Bool(true))
        }
        fn run_microtasks(&mut self) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn interrupt(&mut self) {}
        fn request_gc(&mut self) {}
        fn shutdown(&mut self) {}
    }

    #[test]
    fn primary_tag_is_exposed_to_javascript() {
        let vue = VueRuntime::new(400, 300, 1.0);
        vue.set_window_tag(VueWindowId::PRIMARY, Some("main".into()))
            .unwrap();
        let current = vue.host_api_registry().call("windowCurrent", &[]).unwrap();
        assert_eq!(
            current.as_object().unwrap().get("tag"),
            Some(&nana_js_engine::HostValue::string("main"))
        );
    }

    #[cfg(feature = "dev-reload")]
    #[test]
    fn artifact_reload_after_primary_close_keeps_surviving_windows() {
        let vue = VueRuntime::new(400, 300, 1.0);
        vue.host_api_registry().call("windowCreate", &[]).unwrap();
        vue.request_close(VueWindowId::PRIMARY).unwrap();
        vue.notify_window_closed(VueWindowId::PRIMARY).unwrap();
        vue.drain_runtime_window_commands();
        let survivors = vue.window_ids();
        let mut runtime = VueHostedRuntime {
            engine: InputEngine::default(),
            vue,
            application_api: HostApiRegistry::new(),
        };
        let artifact = RuntimeArtifact::from_source("reload.js", "");
        let replace_engine = || -> InputEngine { panic!("reload tore down the surviving runtime") };
        assert!(
            runtime
                .dev_reload(
                    &artifact,
                    &replace_engine,
                    None,
                    None,
                    ThemeAppearance::Light
                )
                .is_err()
        );
        assert_eq!(runtime.vue.window_ids(), survivors);
        assert!(runtime.vue.drain_runtime_window_commands().is_empty());
    }

    /// Supports realms; records which realm each invoked function belongs to.
    #[derive(Default)]
    struct RealmEngine {
        next_realm: u64,
        functions: Vec<nana_js_engine::JsRealmId>,
        invoked: Vec<nana_js_engine::JsRealmId>,
        disposed: Vec<nana_js_engine::JsRealmId>,
        fail_microtasks: bool,
    }

    impl JsEngine for RealmEngine {
        fn initialize(&mut self, _: RuntimeArtifact) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn register_host_api(&mut self, _: &HostApiRegistry) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn resolve_function(
            &mut self,
            name: &str,
        ) -> Result<nana_js_engine::JsFunctionId, JsEngineError> {
            self.resolve_function_in(nana_js_engine::JsRealmId::MAIN, name)
        }
        fn invoke(
            &mut self,
            target: nana_js_engine::JsFunctionId,
            _: &[nana_js_engine::HostValue],
        ) -> Result<nana_js_engine::HostValue, JsEngineError> {
            let realm = self
                .functions
                .get(target.0 as usize)
                .copied()
                .ok_or_else(|| JsEngineError::new("unknown function"))?;
            self.invoked.push(realm);
            Ok(nana_js_engine::HostValue::Bool(true))
        }
        fn run_microtasks(&mut self) -> Result<(), JsEngineError> {
            if self.fail_microtasks {
                Err(JsEngineError::new("a window-closed listener threw"))
            } else {
                Ok(())
            }
        }
        fn create_realm(&mut self) -> Result<nana_js_engine::JsRealmId, JsEngineError> {
            self.next_realm += 1;
            Ok(nana_js_engine::JsRealmId(self.next_realm))
        }
        fn dispose_realm(&mut self, realm: nana_js_engine::JsRealmId) -> Result<(), JsEngineError> {
            self.disposed.push(realm);
            Ok(())
        }
        fn register_host_api_in(
            &mut self,
            _: nana_js_engine::JsRealmId,
            _: &HostApiRegistry,
        ) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn initialize_in(
            &mut self,
            _: nana_js_engine::JsRealmId,
            _: RuntimeArtifact,
        ) -> Result<(), JsEngineError> {
            Ok(())
        }
        fn resolve_function_in(
            &mut self,
            realm: nana_js_engine::JsRealmId,
            _: &str,
        ) -> Result<nana_js_engine::JsFunctionId, JsEngineError> {
            self.functions.push(realm);
            Ok(nana_js_engine::JsFunctionId(
                self.functions.len() as u64 - 1,
            ))
        }
        fn interrupt(&mut self) {}
        fn request_gc(&mut self) {}
        fn shutdown(&mut self) {}
    }

    fn window_geometry() -> WindowGeometry {
        WindowGeometry {
            physical_position: None,
            physical_size: (320, 240),
            logical_position: None,
            logical_size: (320.0, 240.0),
            scale_factor: 1.0,
            maximized: false,
        }
    }

    fn isolated_create_request() -> nana_js_engine::HostValue {
        nana_js_engine::HostValue::Object(
            [(
                "isolation".into(),
                nana_js_engine::HostValue::string("isolated"),
            )]
            .into_iter()
            .collect(),
        )
    }

    /// Window 1 open in an isolated realm that has evaluated its script.
    fn runtime_with_isolated_window() -> VueHostedRuntime<RealmEngine> {
        let mut engine = RealmEngine::default();
        let mut vue = VueRuntime::new(400, 300, 1.0);
        vue.initialize(
            &mut engine,
            RuntimeArtifact::from_source("app.js", "void 0;"),
            &HostApiRegistry::new(),
        )
        .unwrap();
        vue.host_api_registry()
            .call("windowCreate", &[isolated_create_request()])
            .unwrap();
        let mut runtime = VueHostedRuntime {
            engine,
            vue,
            application_api: HostApiRegistry::new(),
        };
        runtime
            .prepare_window_creation(WindowId(1), window_geometry())
            .unwrap();
        runtime
    }

    #[test]
    fn closing_isolated_window_disposes_its_realm_even_when_delivery_throws() {
        let mut runtime = runtime_with_isolated_window();
        runtime.engine.fail_microtasks = true;
        assert!(
            runtime
                .handle_window_event(WindowEvent::Closed { id: WindowId(1) })
                .is_err()
        );
        assert_eq!(runtime.engine.disposed, [nana_js_engine::JsRealmId(1)]);
    }

    #[cfg(feature = "dev-reload")]
    #[test]
    fn dev_reload_unbinds_closing_isolated_windows_from_the_engine() {
        let mut runtime = runtime_with_isolated_window();
        runtime.vue.dev_close_auxiliary_windows();
        runtime.engine.invoked.clear();
        runtime.pump().unwrap();
        assert!(
            runtime
                .engine
                .invoked
                .iter()
                .all(|realm| *realm == nana_js_engine::JsRealmId::MAIN),
            "a closing isolated window still invoked functions of the replaced engine"
        );
    }

    #[test]
    fn isolated_window_fails_creation_on_engine_without_realms() {
        let mut engine = InputEngine::default();
        let mut vue = VueRuntime::new(400, 300, 1.0);
        vue.initialize(
            &mut engine,
            RuntimeArtifact::from_source("app.js", "void 0;"),
            &HostApiRegistry::new(),
        )
        .unwrap();
        vue.host_api_registry()
            .call("windowCreate", &[isolated_create_request()])
            .unwrap();
        let mut runtime = VueHostedRuntime {
            engine,
            vue,
            application_api: HostApiRegistry::new(),
        };

        assert!(
            runtime
                .prepare_window_creation(WindowId(1), window_geometry())
                .is_err()
        );
        assert_eq!(runtime.vue.window_ids(), [VueWindowId::PRIMARY]);
    }

    #[test]
    fn input_redraw_tracks_noop_single_and_cross_window_changes() {
        let vue = VueRuntime::new(400, 300, 1.0);
        for _ in 0..2 {
            vue.host_api_registry().call("windowCreate", &[]).unwrap();
        }
        let ids = vue.window_ids();
        let mut bridges = Vec::new();
        for id in &ids {
            let host = vue.host(*id).unwrap();
            let mut host = host.lock().unwrap();
            host.callbacks.fire_event = Some(nana_js_engine::JsFunctionId(1));
            let bridge = host.bridge();
            bridge
                .lock()
                .unwrap()
                .set_preset_theme(ThemeAppearance::Light);
            bridges.push(bridge);
        }
        let mut runtime = VueHostedRuntime {
            engine: InputEngine::default(),
            vue,
            application_api: HostApiRegistry::new(),
        };
        for id in &ids {
            runtime
                .vue
                .host(*id)
                .unwrap()
                .lock()
                .unwrap()
                .pump_frame(&mut runtime.engine)
                .unwrap();
        }
        let event = InputPayload::Key(nana_ui_platform::KeyInput::named(
            "F1",
            "F1",
            nana_ui_platform::KeyState::Pressed,
            Default::default(),
        ));
        assert_eq!(
            runtime
                .runtime_input(WindowId::PRIMARY, event.clone())
                .unwrap()
                .redraw,
            RuntimeRedraw::None
        );
        let changed = bridges[1].clone();
        runtime.engine.on_event = Some(Box::new(move || {
            changed
                .lock()
                .unwrap()
                .set_preset_theme(ThemeAppearance::Dark)
        }));
        assert_eq!(
            runtime
                .runtime_input(WindowId::PRIMARY, event.clone())
                .unwrap()
                .redraw,
            RuntimeRedraw::Window(WindowId(ids[1].0))
        );
        let changed = [bridges[0].clone(), bridges[2].clone()];
        runtime.engine.on_event = Some(Box::new(move || {
            for bridge in changed {
                bridge
                    .lock()
                    .unwrap()
                    .set_preset_theme(ThemeAppearance::Dark);
            }
        }));
        assert_eq!(
            runtime
                .runtime_input(WindowId::PRIMARY, event.clone())
                .unwrap()
                .redraw,
            RuntimeRedraw::for_windows([WindowId(ids[0].0), WindowId(ids[2].0)])
        );
    }

    #[test]
    fn input_redraw_tracks_consumed_canvas_and_live_texture_handles() {
        use nana_ui::{
            GpuTextureDescriptor, GpuTextureFormat, GpuTextureUsages, HostTexture,
            HostTextureAlphaMode,
        };
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let gpu = nana_gpu::__framework::adopt(adapter, device, queue);
        let gpu_texture = || {
            gpu.create_texture(&GpuTextureDescriptor {
                label: Some("input redraw regression"),
                width: 8,
                height: 8,
                format: GpuTextureFormat::RGBA8_UNORM,
                usage: GpuTextureUsages::SAMPLED,
            })
            .unwrap()
        };
        let mut runtime = VueHostedRuntime {
            engine: InputEngine::default(),
            vue: VueRuntime::new(400, 300, 1.0),
            application_api: HostApiRegistry::new(),
        };
        runtime
            .vue
            .host_api_registry()
            .call("windowCreate", &[])
            .unwrap();
        let secondary = runtime
            .vue
            .window_ids()
            .into_iter()
            .find(|id| *id != VueWindowId::PRIMARY)
            .unwrap();
        let host = runtime.vue.host(secondary).unwrap();
        let host = host.lock().unwrap();
        let canvas = host.canvas_runtime_ref().clone();
        let id = canvas.lock().unwrap().create_canvas(8, 8).unwrap();
        let detached_id = canvas.lock().unwrap().create_canvas(8, 8).unwrap();
        let registry = host.host_textures().clone();
        let texture = HostTexture::new(7, 1, &gpu_texture());
        registry.register(
            "secondary",
            texture.clone(),
            8,
            8,
            HostTextureAlphaMode::Opaque,
        );
        let document = host.document();
        let mut document = document.lock().unwrap();
        let detached = document.create_element("canvas");
        document.set_attribute(detached, "data-nana-canvas", &detached_id.0.to_string());
        for (tag, attr, value) in [
            ("canvas", "data-nana-canvas", id.0.to_string()),
            ("nana-gpu", "data-nana-gpu", "secondary".into()),
        ] {
            let node = document.create_element(tag);
            let root = document.mount_root();
            document.insert(node, root, None);
            document.set_attribute(node, attr, &value);
        }
        drop(document);
        drop(host);
        for id in runtime.vue.window_ids() {
            let host = runtime.vue.host(id).unwrap();
            let mut host = host.lock().unwrap();
            host.callbacks.fire_event = Some(nana_js_engine::JsFunctionId(1));
            host.pump_frame(&mut runtime.engine).unwrap();
        }
        let event = InputPayload::Key(nana_ui_platform::KeyInput::named(
            "F1",
            "F1",
            nana_ui_platform::KeyState::Pressed,
            Default::default(),
        ));
        let consumed = canvas.clone();
        runtime.engine.on_event = Some(Box::new(move || {
            consumed.lock().unwrap().resize_canvas(id, 8, 8).unwrap()
        }));
        assert_eq!(
            runtime
                .runtime_input(WindowId::PRIMARY, event.clone())
                .unwrap()
                .redraw,
            RuntimeRedraw::Window(WindowId(secondary.0))
        );
        let unused = canvas.clone();
        runtime.engine.on_event = Some(Box::new(move || {
            unused.lock().unwrap().create_canvas(8, 8).unwrap();
        }));
        assert_eq!(
            runtime
                .runtime_input(WindowId::PRIMARY, event.clone())
                .unwrap()
                .redraw,
            RuntimeRedraw::None
        );
        let detached = canvas.clone();
        runtime.engine.on_event = Some(Box::new(move || {
            detached
                .lock()
                .unwrap()
                .resize_canvas(detached_id, 16, 16)
                .unwrap();
        }));
        assert_eq!(
            runtime
                .runtime_input(WindowId::PRIMARY, event.clone())
                .unwrap()
                .redraw,
            RuntimeRedraw::None
        );
        runtime.engine.on_event = Some(Box::new(move || {
            texture.invalidate();
        }));
        assert_eq!(
            runtime
                .runtime_input(WindowId::PRIMARY, event.clone())
                .unwrap()
                .redraw,
            RuntimeRedraw::Window(WindowId(secondary.0))
        );
        let replacement = HostTexture::new(7, 1, &gpu_texture());
        replacement.invalidate();
        runtime.engine.on_event = Some(Box::new(move || {
            registry.register("secondary", replacement, 8, 8, HostTextureAlphaMode::Opaque);
        }));
        assert_eq!(
            runtime
                .runtime_input(WindowId::PRIMARY, event.clone())
                .unwrap()
                .redraw,
            RuntimeRedraw::Window(WindowId(secondary.0))
        );
    }

    #[test]
    fn vue_window_documents_are_the_same_runtime_tree() {
        let host = VueHost::new();
        let shared = host.shared_runtime_document();
        let facade = host.document();
        let guard = facade.lock().unwrap();
        assert_eq!(
            shared.with_document(|_| ()),
            Err(nana_ui_scene::DocumentAccessError::Busy)
        );
        drop(guard);
        assert_eq!(
            shared
                .with_document(|document| document.document())
                .unwrap(),
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

    fn pointer_down(x: f32, y: f32) -> InputPayload {
        InputPayload::Pointer(nana_ui_platform::PointerInput {
            pressure: 0.0,
            ..nana_ui_platform::PointerInput::mouse(PointerPhase::Down, x, y)
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn pointer_move(x: f32, y: f32) -> InputPayload {
        InputPayload::Pointer(nana_ui_platform::PointerInput {
            buttons: 1,
            pressure: 0.0,
            ..nana_ui_platform::PointerInput::mouse(PointerPhase::Move, x, y)
        })
    }

    fn vue_app_shell_document() -> (
        crate::NanaTreeDocument,
        nana_ui_runtime::StableNodeId,
        nana_ui_runtime::StableNodeId,
    ) {
        use crate::{MessageBridge, NanaTreeDocument, WidgetKind, WidgetProps};
        use nana_ui_runtime::{LayoutViewport, StableNodeId};

        let mut doc = NanaTreeDocument::new(800, 600, 1.0);
        let shell = doc.create_element("nana-app-shell");
        let title_bar = doc.create_element("nana-app-title-bar");
        let button = doc.create_element("button");
        let body = doc.create_element("div");
        doc.insert(shell, doc.mount_root(), None);
        doc.insert(title_bar, shell, None);
        doc.insert(button, title_bar, None);
        doc.insert(body, shell, None);

        let mut title_props = WidgetProps {
            label: "Nana".into(),
            element_tag: "nana-app-title-bar".into(),
            ..Default::default()
        };
        title_props
            .attrs
            .insert("data-slot".into(), "title-bar".into());
        title_props.class_names.push("nana-app-title-bar".into());
        let mut button_props = WidgetProps::default();
        button_props.element_tag = "button".into();
        button_props.label = "Close".into();

        let mut bridge = MessageBridge::new();
        bridge.register(title_bar.0, WidgetKind::Column, title_props);
        bridge.register(button.0, WidgetKind::Button, button_props);
        bridge.register(
            body.0,
            WidgetKind::Column,
            WidgetProps {
                label: "Workspace".into(),
                ..Default::default()
            },
        );
        bridge.register(
            shell.0,
            WidgetKind::AppShell,
            WidgetProps {
                label: "Nana".into(),
                ..Default::default()
            },
        );
        bridge.insert_child(title_bar.0, shell.0, None);
        bridge.insert_child(button.0, title_bar.0, None);
        bridge.insert_child(body.0, shell.0, None);
        doc.sync_semantic_styles(&bridge.snapshot());
        let document = doc.runtime_document().document();
        doc.context_mut()
            .layout_document(document, LayoutViewport::new(800.0, 600.0))
            .unwrap();
        doc.context_mut().rebuild_hit_test(document);
        (
            doc,
            StableNodeId::try_from(title_bar).unwrap(),
            StableNodeId::try_from(button).unwrap(),
        )
    }

    #[test]
    fn vue_app_shell_title_bar_blank_emits_drag_and_skips_buttons() {
        let (doc, title, button) = vue_app_shell_document();
        let runtime = doc.runtime_document();
        let document = runtime.document();
        let context = runtime.context();
        let bounds = context.world().layout_box(title).unwrap();
        let blank_x = bounds.x + bounds.width / 2.0;
        let blank_y = bounds.y + bounds.height / 2.0;

        let mut state = WindowChromeState::default();
        let mut tracker = TitleBarDragTracker::default();
        let pressed = apply_title_bar_pointer(
            &mut state,
            &mut tracker,
            context,
            document,
            &pointer_down(blank_x, blank_y),
        );
        #[cfg(target_os = "macos")]
        assert_eq!(pressed, Some(WindowChromeAction::Drag));
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(pressed, None);
            assert_eq!(
                apply_title_bar_pointer(
                    &mut state,
                    &mut tracker,
                    context,
                    document,
                    &pointer_move(blank_x + 8.0, blank_y),
                ),
                Some(WindowChromeAction::Drag)
            );
        }

        #[cfg(not(target_os = "macos"))]
        {
            let mut stack = context.world().node(title).unwrap().children;
            let mut controls = None;
            while let Some(id) = stack.pop() {
                if matches!(
                    context.world().node(id).map(|node| node.kind),
                    Some(nana_ui_runtime::NodeKind::Element { tag })
                        if tag.contains("title-bar-controls")
                ) {
                    controls = Some(id);
                    break;
                }
                if let Some(children) = context.world().node(id).map(|node| node.children) {
                    stack.extend(children);
                }
            }
            let controls = controls.expect("Windows/Linux title bar must assemble window controls");
            assert_eq!(context.world().node(controls).unwrap().children.len(), 3);
        }

        let button_box = context.world().layout_box(button).unwrap();
        let mut control_state = WindowChromeState::default();
        let mut control_tracker = TitleBarDragTracker::default();
        assert_eq!(
            apply_title_bar_pointer(
                &mut control_state,
                &mut control_tracker,
                context,
                document,
                &pointer_down(
                    button_box.x + button_box.width / 2.0,
                    button_box.y + button_box.height / 2.0,
                ),
            ),
            None
        );
    }
}
