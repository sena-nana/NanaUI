//! Nana-owned winit + wgpu loop for [`crate::RuntimeProgram`].
//!
//! Paint goes through [`crate::SceneWgpuPainter`].

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nana_ui_core::AppearanceSettings;
use nana_ui_platform::{
    ImeEvent, InputEvent, InputModifiers, PointerPhase, PointerType, TextInputPurpose,
    TextInputRequest, WindowCommand, WindowEvent, WindowGeometry, WindowId,
};
use nana_ui_runtime::{
    AccessibilityUpdate, AppTitleBar, Entity, FrameworkError, LayoutViewport, Task,
};
use nana_window::{
    Appearance, FallbackColor, MaterialEffect, MaterialOutcome, apply_hosted_system_material,
    clear_system_material, prepare_client_chrome,
};
use winit::application::ApplicationHandler;
use winit::event::{
    ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent as WinitWindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;
#[cfg(target_os = "windows")]
use winit::platform::windows::{WindowAttributesExtWindows, WindowExtWindows};
#[cfg(target_os = "windows")]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[cfg(not(target_os = "android"))]
use crate::accessibility::HostedAccessibility;
use crate::nana_text::NanaTextShaper;
use crate::runtime_host::{
    HostFailure, RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw,
    RuntimeWindowSettings, gated_runtime_window_update, runtime_text_input_request,
};
use crate::scene_paint::{ScenePaintViewport, SceneWgpuPainter};
use crate::{
    HostTextureRegistry, HostedGpuContext, HostedGpuError, HostedGpuSurface, HostedRunError,
    HostedSurfaceFrame, RuntimeAnimationClock, RuntimeInputAdapter, SceneGpuRendererRegistry,
    TitleBarDragTracker, WindowChromeAction, WindowChromeEvent, WindowChromeState,
    apply_title_bar_pointer, default_scene_gpu_renderers_with_host, resolve_scene_gpu_renderers,
    window_commands_for_chrome_action,
};

const GPU_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const TASK_QUEUE_CAPACITY: usize = 256;
const TASK_WORKERS: usize = 4;

/// Run a [`RuntimeProgram`] on the Nana Scene host.
pub fn run_runtime_scene<Program: RuntimeProgram>(
    settings: RuntimeWindowSettings,
) -> Result<(), HostedRunError> {
    let event_loop = EventLoop::<Program::Message>::with_user_event()
        .build()
        .map_err(HostedRunError::EventLoop)?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut runner = SceneRunner::<Program>::Loading {
        proxy: event_loop.create_proxy(),
        settings,
        failure: None,
    };
    event_loop
        .run_app(&mut runner)
        .map_err(HostedRunError::EventLoop)?;
    runner.into_result()
}

enum SceneRunner<Program: RuntimeProgram> {
    Loading {
        proxy: EventLoopProxy<Program::Message>,
        settings: RuntimeWindowSettings,
        failure: Option<String>,
    },
    Ready(Box<SceneReady<Program>>),
    Finished {
        failure: Option<String>,
    },
}

struct SceneAuxiliary {
    surface: HostedGpuSurface,
    geometry: WindowGeometry,
    input: InputTracker,
    material: MaterialOutcome,
    settings: RuntimeWindowSettings,
    #[cfg(not(target_os = "android"))]
    accessibility: Option<HostedAccessibility>,
    accessibility_pending: Option<AccessibilityUpdate>,
    #[cfg(target_os = "windows")]
    pen_hook: crate::windows_pen::WindowsPenHook,
}

impl Drop for SceneAuxiliary {
    fn drop(&mut self) {
        clear_system_material(self.surface.window().as_ref());
    }
}

struct SceneReady<Program: RuntimeProgram> {
    program: Program,
    graphics: HostedGpuContext,
    painters: HashMap<wgpu::TextureFormat, SceneWgpuPainter>,
    text: NanaTextShaper,
    proxy: EventLoopProxy<Program::Message>,
    tasks: SyncSender<Task<Program::Message>>,
    geometry: WindowGeometry,
    animation_clock: RuntimeAnimationClock,
    default_scene_gpu_renderers: Option<SceneGpuRendererRegistry>,
    #[cfg(not(target_os = "android"))]
    accessibility: Option<HostedAccessibility>,
    accessibility_pending: Option<AccessibilityUpdate>,
    input: InputTracker,
    material: MaterialOutcome,
    auxiliary: HashMap<WindowId, SceneAuxiliary>,
    window_ids: HashMap<winit::window::WindowId, WindowId>,
    #[cfg(target_os = "windows")]
    pen_hook: crate::windows_pen::WindowsPenHook,
    next_gpu_retry: Option<Instant>,
    render_suspended: bool,
    last_theme: crate::ThemeMode,
    last_material_mode: nana_window::MaterialEffect,
    settings: RuntimeWindowSettings,
    ime_requests: HashMap<WindowId, TextInputRequest>,
    chrome: HashMap<WindowId, WindowChromeSession>,
}

struct WindowChromeSession {
    state: WindowChromeState,
    drag: TitleBarDragTracker,
}

impl WindowChromeSession {
    fn new(id: WindowId) -> Self {
        Self {
            state: WindowChromeState::for_window(id, crate::WindowChrome::platform_default()),
            drag: TitleBarDragTracker::default(),
        }
    }
}

impl<Program: RuntimeProgram> SceneRunner<Program> {
    fn into_result(self) -> Result<(), HostedRunError> {
        let failure = match self {
            Self::Loading { failure, .. } | Self::Finished { failure } => failure,
            Self::Ready(_) => None,
        };
        failure.map_or(Ok(()), |message| Err(HostedRunError::Startup(message)))
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: impl Into<String>) {
        *self = Self::Finished {
            failure: Some(message.into()),
        };
        event_loop.exit();
    }
}

impl<Program: RuntimeProgram> ApplicationHandler<Program::Message> for SceneRunner<Program> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Loading {
            proxy,
            settings,
            failure,
        } = self
        else {
            return;
        };
        if failure.is_some() {
            event_loop.exit();
            return;
        }
        match initialize::<Program>(event_loop, proxy.clone(), settings.clone()) {
            Ok(ready) => *self = Self::Ready(Box::new(ready)),
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, message: Program::Message) {
        let Self::Ready(ready) = self else {
            return;
        };
        ready.process_message(event_loop, message);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WinitWindowEvent,
    ) {
        let Self::Ready(ready) = self else {
            return;
        };
        let Some(id) = ready.window_ids.get(&window_id).copied() else {
            return;
        };
        ready.handle_window_event(event_loop, id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Ready(ready) = self else {
            return;
        };
        ready.about_to_wait(event_loop);
    }
}

fn initialize<Program: RuntimeProgram>(
    event_loop: &ActiveEventLoop,
    proxy: EventLoopProxy<Program::Message>,
    settings: RuntimeWindowSettings,
) -> Result<SceneReady<Program>, String> {
    let window = Arc::new(
        event_loop
            .create_window(scene_window_attributes(&settings).with_visible(false))
            .map_err(|error| format!("failed to create scene window: {error}"))?,
    );
    if !settings.system_caption {
        let _ = prepare_client_chrome(window.as_ref());
    }
    let mut last_theme = crate::ThemeMode::default();
    let mut last_material_mode = nana_window::MaterialEffect::Solid;
    let mut material = apply_window_surface(
        window.as_ref(),
        last_theme,
        settings.transparent,
        last_material_mode,
    );
    let mut graphics = pollster::block_on(HostedGpuContext::new(
        Arc::clone(&window),
        wgpu::Features::empty(),
        window_wants_transparent_surface(settings.transparent, last_material_mode),
    ))
    .map_err(|error| error.to_string())?;
    let format = graphics.format();
    let mut painters = HashMap::new();
    painters.insert(
        format,
        SceneWgpuPainter::new(
            graphics.resources().device(),
            graphics.resources().queue(),
            format,
        ),
    );
    let tasks = spawn_task_workers(proxy.clone());
    let geometry = window_geometry(graphics.window());
    let context = program_context(
        &proxy,
        &graphics,
        WindowId::PRIMARY,
        geometry,
        tasks.clone(),
        material,
    );
    let (program, startup) = Program::initialize(&context).map_err(|error| error.to_string())?;
    let default_scene_gpu_renderers = Some(default_scene_gpu_renderers_with_host(
        Arc::clone(graphics.resources().device()),
        Arc::clone(graphics.resources().queue()),
    ));
    last_theme = program.theme_mode();
    last_material_mode = program.window_material_mode();
    material = apply_window_surface(
        graphics.window().as_ref(),
        last_theme,
        settings.transparent,
        last_material_mode,
    );
    graphics.reconfigure_alpha_mode(window_wants_transparent_surface(
        settings.transparent,
        last_material_mode,
    ));
    #[cfg(not(target_os = "android"))]
    let accessibility = {
        Some(HostedAccessibility::new(
            event_loop,
            Arc::clone(graphics.window()),
            accessibility_world_generation(&program, WindowId::PRIMARY),
            accessibility_snapshot(&program, WindowId::PRIMARY),
            true,
            window.scale_factor() as f32,
        ))
    };
    let mut window_ids = HashMap::new();
    window_ids.insert(window.id(), WindowId::PRIMARY);
    let mut ready = SceneReady {
        program,
        graphics,
        painters,
        text: NanaTextShaper::default(),
        proxy,
        tasks,
        geometry,
        animation_clock: RuntimeAnimationClock::now(),
        default_scene_gpu_renderers,
        #[cfg(not(target_os = "android"))]
        accessibility,
        accessibility_pending: None,
        input: InputTracker::default(),
        material,
        auxiliary: HashMap::new(),
        window_ids,
        #[cfg(target_os = "windows")]
        pen_hook: crate::windows_pen::WindowsPenHook::install(window.as_ref())?,
        next_gpu_retry: None,
        render_suspended: false,
        last_theme,
        last_material_mode,
        settings,
        ime_requests: HashMap::new(),
        chrome: HashMap::new(),
    };
    ready.prepare_window_chrome(WindowId::PRIMARY, ready.geometry.maximized);
    let update = ready.program.window_event(
        WindowEvent::Ready {
            id: WindowId::PRIMARY,
            geometry: ready.geometry,
        },
        &ready.context(),
    );
    ready.apply_update(event_loop, update);
    for message in startup {
        if event_loop.exiting() {
            break;
        }
        ready.process_message(event_loop, message);
    }
    if !event_loop.exiting() {
        ready.graphics.window().set_visible(true);
        ready.graphics.window().request_redraw();
    }
    Ok(ready)
}

impl<Program: RuntimeProgram> SceneReady<Program> {
    fn context(&self) -> RuntimeProgramContext<Program::Message> {
        self.context_for(WindowId::PRIMARY)
    }

    fn context_for(&self, id: WindowId) -> RuntimeProgramContext<Program::Message> {
        program_context(
            &self.proxy,
            &self.graphics,
            id,
            self.geometry_of(id),
            self.tasks.clone(),
            self.material_of(id),
        )
    }

    fn process_message(&mut self, event_loop: &ActiveEventLoop, message: Program::Message) {
        let update = self.program.update(message, &self.context());
        self.sync_appearance();
        self.apply_update(event_loop, update);
    }

    fn handle_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        event: WinitWindowEvent,
    ) {
        #[cfg(not(target_os = "android"))]
        if let Some(window) = self.window(id).cloned()
            && let Some(accessibility) = self.accessibility_mut(id)
        {
            accessibility.process_event(window.as_ref(), &event);
        }
        #[cfg(not(target_os = "android"))]
        for request in self.take_accessibility_actions(id) {
            let update = match self
                .program
                .accessibility_action(id, request, &self.context_for(id))
            {
                Ok(update) => update,
                Err(error) => {
                    self.program.host_failure(HostFailure::AccessibilityAction {
                        window: id,
                        error: error.to_string(),
                    });
                    continue;
                }
            };
            self.apply_update(event_loop, update);
            if event_loop.exiting() {
                return;
            }
        }
        if let Some(modal) = self.active_modal_child(id)
            && !allows_modal_parent_event(&event)
        {
            #[cfg(target_os = "windows")]
            let _ = self.take_windows_pen_events(id);
            self.focus_window(modal);
            return;
        }
        #[cfg(target_os = "windows")]
        self.dispatch_windows_pen_events(event_loop, id);
        if let WinitWindowEvent::ModifiersChanged(modifiers) = &event {
            self.input_mut(id).modifiers = modifiers.state();
        }
        if let WinitWindowEvent::CursorMoved { position, .. } = &event {
            let scale = self.scale_factor(id);
            let point = position.to_logical::<f32>(f64::from(scale));
            self.input_mut(id).cursor = (point.x, point.y);
            self.sync_window_cursor(id);
        }
        if let Some(input) = self.normalized_input(id, &event) {
            let disposition = self.dispatch_input(event_loop, id, input);
            if disposition.prevent_default || event_loop.exiting() {
                return;
            }
        }
        match &event {
            WinitWindowEvent::RedrawRequested => self.redraw(event_loop, id),
            WinitWindowEvent::CloseRequested if id == WindowId::PRIMARY => {
                self.forward_window_event(event_loop, id, &event);
                event_loop.exit();
            }
            WinitWindowEvent::CloseRequested => {
                self.forward_window_event(event_loop, id, &event);
                if self.auxiliary.contains_key(&id) {
                    self.close_window(event_loop, id);
                }
            }
            WinitWindowEvent::Destroyed if id == WindowId::PRIMARY => {
                self.forward_window_event(event_loop, id, &event);
                event_loop.exit();
            }
            WinitWindowEvent::Destroyed => {
                if self.auxiliary.contains_key(&id) {
                    self.close_window(event_loop, id);
                }
            }
            WinitWindowEvent::Moved(_) => {
                self.sync_geometry(id);
                self.forward_window_event(event_loop, id, &event);
            }
            WinitWindowEvent::Resized(_) | WinitWindowEvent::ScaleFactorChanged { .. } => {
                self.resize_window(id);
                self.sync_geometry(id);
                self.forward_window_event(event_loop, id, &event);
                self.request_redraw(id);
            }
            WinitWindowEvent::Occluded(_) => {
                self.forward_window_event(event_loop, id, &event);
            }
            WinitWindowEvent::Focused(focused) => {
                if !*focused {
                    self.input_mut(id).clear_pointers();
                }
                self.forward_window_event(event_loop, id, &event);
                self.apply_ime_request(id);
            }
            WinitWindowEvent::Ime(ime) => {
                self.handle_ime(event_loop, id, platform_ime_event(ime.clone()))
            }
            WinitWindowEvent::HoveredFile(_)
            | WinitWindowEvent::DroppedFile(_)
            | WinitWindowEvent::HoveredFileCancelled => {
                if let Some(window_event) = self.input_mut(id).map_file_window_event(&event, id) {
                    let update = self
                        .program
                        .window_event(window_event, &self.context_for(id));
                    self.apply_update(event_loop, update);
                }
            }
            _ => {}
        }
    }

    fn forward_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        event: &WinitWindowEvent,
    ) {
        if let Some(window_event) = platform_window_event(event, id, self.geometry_of(id)) {
            let update = self
                .program
                .window_event(window_event, &self.context_for(id));
            self.apply_update(event_loop, update);
        }
    }

    fn handle_ime(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: ImeEvent) {
        let window_event = WindowEvent::Ime {
            id,
            event: event.clone(),
        };
        let ime_changed = self
            .program
            .document_mut(id)
            .map(|document| {
                let document_id = document.document();
                RuntimeInputAdapter::default()
                    .dispatch_ime(document.context_mut(), document_id, &event)
                    .map(|disposition| {
                        disposition.prevent_default && !matches!(event, ImeEvent::Enabled)
                    })
            })
            .transpose()
            .unwrap_or_else(|error| {
                // Drop this IME event instead of panicking; the program sees
                // the failure through host_failure.
                self.program.host_failure(HostFailure::ImeDispatch {
                    window: id,
                    error: error.to_string(),
                });
                Some(false)
            })
            .unwrap_or(false);
        let modal_blocks_ime = self.program.document(id).is_some_and(|document| {
            document
                .context()
                .has_blocking_runtime_overlay(document.document())
        });
        // Runtime already applied IME. Still notify the program so Vue can emit
        // JS events; programs must not re-apply the same IME to Runtime.
        let mut update =
            gated_runtime_window_update(!should_deliver_program_ime(modal_blocks_ime), || {
                self.program
                    .window_event(window_event, &self.context_for(id))
            });
        if ime_changed {
            update = update.merge(RuntimeProgramUpdate::redraw(id));
        }
        self.sync_appearance();
        self.apply_update(event_loop, update);
        self.apply_ime_request(id);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.graphics.take_device_lost()
            || self.next_gpu_retry.is_some_and(|deadline| now >= deadline)
        {
            self.recover_device(event_loop);
        }
        if self.next_wakeup().is_some_and(|deadline| now >= deadline) {
            self.wake(event_loop, now);
        }
        let next_wakeup = [self.next_gpu_retry, self.next_wakeup()]
            .into_iter()
            .flatten()
            .min();
        event_loop.set_control_flow(next_wakeup.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn animation_deadline(&self) -> Option<Instant> {
        self.known_window_ids()
            .into_iter()
            .filter_map(|id| {
                self.program
                    .document(id)
                    .and_then(|document| self.animation_clock.next_wakeup(document.context()))
            })
            .min()
    }

    fn next_wakeup(&self) -> Option<Instant> {
        match (self.animation_deadline(), self.program.next_wakeup()) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
            (None, None) => None,
        }
    }

    fn wake(&mut self, event_loop: &ActiveEventLoop, now: Instant) {
        let mut update = self.program.wake(now, &self.context());
        for id in self.known_window_ids() {
            let frame = self
                .program
                .document_mut(id)
                .map(|document| self.animation_clock.wake(document.context_mut(), now));
            let Some(frame) = frame else {
                continue;
            };
            let had_samples = frame.has_updates();
            match self
                .program
                .animation_frame(id, frame, &self.context_for(id))
            {
                Ok(frame_update) => update = update.merge(frame_update),
                Err(error) => {
                    self.program.host_failure(HostFailure::AnimationFrame {
                        window: id,
                        error: error.to_string(),
                    });
                }
            }
            if had_samples {
                update = update.merge(RuntimeProgramUpdate::redraw(id));
            }
        }
        self.sync_appearance();
        self.apply_update(event_loop, update);
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        if self.render_suspended {
            return;
        }
        if self.graphics.take_device_lost() {
            self.recover_device(event_loop);
            return;
        }
        if id != WindowId::PRIMARY && !self.auxiliary.contains_key(&id) {
            return;
        }
        self.program.prepare_window_frame(id, &self.context_for(id));
        let geometry = self.geometry_of(id);
        let material = self.material_of(id);
        let viewport = LayoutViewport::new(geometry.logical_size.0, geometry.logical_size.1);
        let flush = {
            let Some(document) = self.program.document_mut(id) else {
                // prepare_window_frame ran program code that may have closed
                // this window's document; skip the frame instead of panicking.
                self.program
                    .host_failure(HostFailure::MissingDocument { window: id });
                return;
            };
            document.flush(viewport, &mut self.text)
        };
        let update = match flush {
            Ok(update) => update,
            Err(error) => {
                // The frame did not settle; Runtime restored its dirty work,
                // so the next redraw retries. Skipping keeps the process alive.
                self.program.host_failure(HostFailure::FrameDidNotSettle {
                    window: id,
                    error: error.to_string(),
                });
                return;
            }
        };
        let pending = if !update.accessibility.updated.is_empty()
            || !update.accessibility.removed.is_empty()
        {
            Some(AccessibilityUpdate::Delta(update.accessibility))
        } else {
            None
        };
        let scene = {
            let Some(document) = self.program.document_mut(id) else {
                self.program
                    .host_failure(HostFailure::MissingDocument { window: id });
                return;
            };
            document.shared_scene()
        };
        if let Some(pending) = pending {
            *self.accessibility_pending_mut(id) = Some(pending);
        }
        if let Some(producers) = self.program.scene_resource_producers(id)
            && let Err(error) = producers.encode_scene(
                scene.as_ref(),
                self.graphics.resources().device(),
                self.graphics.resources().queue(),
            )
        {
            self.program.host_failure(HostFailure::ResourceProduction {
                window: id,
                error: error.to_string(),
            });
            return;
        }
        let format = if id == WindowId::PRIMARY {
            self.graphics.format()
        } else {
            let Some(auxiliary) = self.auxiliary.get(&id) else {
                // prepare_window_frame may have closed this auxiliary surface
                // after the redraw guard above admitted it.
                self.program
                    .host_failure(HostFailure::AuxiliarySurfaceLost { window: id });
                return;
            };
            auxiliary.surface.format()
        };
        let frame = match self.acquire_frame(id) {
            Ok(HostedSurfaceFrame::Ready(frame)) => frame,
            Ok(HostedSurfaceFrame::Retry) => {
                self.request_redraw(id);
                return;
            }
            Ok(HostedSurfaceFrame::Skipped) => return,
            Err(error) => {
                self.suspend_rendering(error);
                return;
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.graphics.resources().device().create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("NanaUI scene host frame"),
            },
        );
        let host_textures = self.program.host_textures(id);
        let gpu_renderers = resolve_scene_gpu_renderers(
            self.program.scene_gpu_renderers(id),
            self.default_scene_gpu_renderers.clone(),
        );
        let theme = self.program.theme_mode();
        let paint = self.painter_mut(format).paint(
            scene.as_ref(),
            &mut encoder,
            &target,
            scene_paint_viewport(&geometry, material, theme),
            host_textures.as_ref(),
            gpu_renderers.as_ref(),
        );
        if let Err(error) = paint {
            self.program.host_failure(HostFailure::UnpaintableScene {
                window: id,
                error: error.to_string(),
            });
            self.request_redraw(id);
            return;
        }
        let submit_started = std::time::Instant::now();
        self.graphics.resources().queue().submit([encoder.finish()]);
        self.painter_mut(format)
            .record_submit(submit_started.elapsed());
        self.graphics.present(frame);
        self.apply_ime_request(id);
        let update = self
            .program
            .window_frame_presented(id, &self.context_for(id));
        self.sync_appearance();
        self.apply_update(event_loop, update);
        #[cfg(not(target_os = "android"))]
        self.synchronize_accessibility(id);
    }

    fn acquire_frame(&mut self, id: WindowId) -> Result<HostedSurfaceFrame, HostedGpuError> {
        if id == WindowId::PRIMARY {
            self.graphics.acquire_frame()
        } else {
            let host = self
                .auxiliary
                .get_mut(&id)
                .ok_or(HostedGpuError::SurfaceValidation)?;
            self.graphics.acquire_surface_frame(&mut host.surface)
        }
    }

    fn apply_update(&mut self, event_loop: &ActiveEventLoop, update: RuntimeProgramUpdate) {
        if update.exit {
            event_loop.exit();
            return;
        }
        for command in update.window_commands {
            self.apply_window_command(event_loop, command);
            if event_loop.exiting() {
                return;
            }
        }
        for id in windows_to_redraw(update.redraw, &self.known_window_ids()) {
            self.request_redraw(id);
        }
    }

    fn apply_window_command(&mut self, event_loop: &ActiveEventLoop, command: WindowCommand) {
        let known = self.known_window_ids();
        match route_window_command(&command, &known) {
            RoutedWindowCommand::Ignore => {}
            RoutedWindowCommand::Open(id) => {
                let WindowCommand::Open { settings, .. } = command else {
                    return;
                };
                if let Ok(event) = self.open_window(event_loop, id, settings) {
                    let update = self.program.window_event(event, &self.context_for(id));
                    self.apply_update(event_loop, update);
                }
            }
            RoutedWindowCommand::Focus(id) => self.focus_window(id),
            RoutedWindowCommand::Close(id) => self.close_window(event_loop, id),
            RoutedWindowCommand::SetTitle(id) => {
                let WindowCommand::SetTitle { title, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    window.set_title(&title);
                }
            }
            RoutedWindowCommand::Move(id) => {
                let WindowCommand::Move { position, .. } = command else {
                    return;
                };
                self.move_window(id, position);
            }
            RoutedWindowCommand::SetBounds(id) => {
                let WindowCommand::SetBounds { position, size, .. } = command else {
                    return;
                };
                self.set_window_bounds(id, position, size);
            }
            RoutedWindowCommand::SetFullscreen(id) => {
                let WindowCommand::SetFullscreen { fullscreen, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    window.set_fullscreen(
                        fullscreen.then_some(winit::window::Fullscreen::Borderless(None)),
                    );
                }
            }
            RoutedWindowCommand::SetMinimized(id) => {
                let WindowCommand::SetMinimized { minimized, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    window.set_minimized(minimized);
                }
            }
            RoutedWindowCommand::SetMaximized(id) => {
                let WindowCommand::SetMaximized { maximized, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id).cloned() {
                    window.set_maximized(maximized);
                    self.resize_window(id);
                    self.sync_geometry(id);
                    let update = self.program.window_event(
                        WindowEvent::Resized {
                            id,
                            geometry: self.geometry_of(id),
                        },
                        &self.context_for(id),
                    );
                    self.apply_update(event_loop, update);
                }
            }
            RoutedWindowCommand::SetAlwaysOnTop(id) => {
                let WindowCommand::SetAlwaysOnTop { always_on_top, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    window.set_window_level(window_level(always_on_top));
                }
            }
            RoutedWindowCommand::Drag(id) => {
                if let Some(window) = self.window(id) {
                    drag_scene_window(window.as_ref());
                }
            }
        }
    }

    fn open_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        settings: RuntimeWindowSettings,
    ) -> Result<WindowEvent, String> {
        if settings.modal {
            let parent = settings
                .parent
                .ok_or_else(|| "modal window requires a parent".to_string())?;
            if self.window(parent).is_none() {
                return Err(format!("modal parent window {} does not exist", parent.0));
            }
            if self.active_modal_child(parent).is_some() {
                return Err(format!(
                    "modal parent window {} is already blocked",
                    parent.0
                ));
            }
        }
        let parent = settings
            .parent
            .and_then(|parent| self.window(parent).cloned());
        let attributes = scene_aux_window_attributes(&settings, parent.as_deref())?;
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| error.to_string())?,
        );
        if !settings.system_caption {
            let _ = prepare_client_chrome(window.as_ref());
        }
        let material = apply_window_surface(
            window.as_ref(),
            self.last_theme,
            settings.transparent,
            self.last_material_mode,
        );
        let surface = self
            .graphics
            .create_surface(
                Arc::clone(&window),
                window_wants_transparent_surface(settings.transparent, self.last_material_mode),
            )
            .map_err(|error| error.to_string())?;
        let format = surface.format();
        let _ = self.painter_mut(format);
        #[cfg(target_os = "windows")]
        let pen_hook = crate::windows_pen::WindowsPenHook::install(window.as_ref())?;
        #[cfg(not(target_os = "android"))]
        let accessibility = {
            Some(HostedAccessibility::new(
                event_loop,
                Arc::clone(&window),
                accessibility_world_generation(&self.program, id),
                accessibility_snapshot(&self.program, id),
                true,
                window.scale_factor() as f32,
            ))
        };
        let geometry = window_geometry(&window);
        #[cfg(target_os = "windows")]
        let modal_parent = settings.modal.then_some(settings.parent).flatten();
        self.window_ids.insert(window.id(), id);
        self.auxiliary.insert(
            id,
            SceneAuxiliary {
                surface,
                geometry,
                input: InputTracker::default(),
                material,
                settings,
                #[cfg(not(target_os = "android"))]
                accessibility,
                accessibility_pending: None,
                #[cfg(target_os = "windows")]
                pen_hook,
            },
        );
        #[cfg(target_os = "windows")]
        if let Some(parent) = modal_parent.and_then(|parent| self.window(parent)) {
            parent.set_enable(false);
        }
        window.set_visible(true);
        window.request_redraw();
        self.prepare_window_chrome(id, geometry.maximized);
        Ok(WindowEvent::Ready { id, geometry })
    }

    fn close_window(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        if id == WindowId::PRIMARY {
            return;
        }
        self.chrome.remove(&id);
        if let Some(host) = self.auxiliary.remove(&id) {
            #[cfg(target_os = "windows")]
            if let Some(parent) = host
                .settings
                .modal
                .then_some(host.settings.parent)
                .flatten()
                .and_then(|parent| self.window(parent))
            {
                parent.set_enable(true);
                parent.focus_window();
            }
            self.window_ids.remove(&host.surface.window().id());
            self.ime_requests.remove(&id);
            drop(host);
            let update = self
                .program
                .window_event(WindowEvent::Closed { id }, &self.context_for(id));
            self.apply_update(event_loop, update);
        }
    }

    fn focus_window(&self, id: WindowId) {
        if let Some(window) = self.window(id) {
            window.set_visible(true);
            window.focus_window();
        }
    }

    fn move_window(&self, id: WindowId, position: (f32, f32)) {
        let Some(window) = self.window(id) else {
            return;
        };
        window.set_outer_position(winit::dpi::Position::Logical(
            winit::dpi::LogicalPosition::new(f64::from(position.0), f64::from(position.1)),
        ));
    }

    fn set_window_bounds(&self, id: WindowId, position: (f32, f32), size: (f32, f32)) {
        let Some(window) = self.window(id) else {
            return;
        };
        window.set_outer_position(winit::dpi::Position::Logical(
            winit::dpi::LogicalPosition::new(f64::from(position.0), f64::from(position.1)),
        ));
        let _ = window.request_inner_size(winit::dpi::Size::Logical(winit::dpi::LogicalSize::new(
            f64::from(size.0.max(1.0)),
            f64::from(size.1.max(1.0)),
        )));
    }

    fn active_modal_child(&self, parent: WindowId) -> Option<WindowId> {
        self.auxiliary.iter().find_map(|(id, host)| {
            (host.settings.modal && host.settings.parent == Some(parent)).then_some(*id)
        })
    }

    fn recover_device(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::clone(self.graphics.window());
        let _ = apply_window_surface(
            window.as_ref(),
            self.last_theme,
            self.settings.transparent,
            self.last_material_mode,
        );
        match pollster::block_on(HostedGpuContext::new(
            window,
            wgpu::Features::empty(),
            window_wants_transparent_surface(self.settings.transparent, self.last_material_mode),
        )) {
            Ok(graphics) => {
                let mut painters = HashMap::new();
                painters.insert(
                    graphics.format(),
                    SceneWgpuPainter::new(
                        graphics.resources().device(),
                        graphics.resources().queue(),
                        graphics.format(),
                    ),
                );
                let previous = std::mem::take(&mut self.auxiliary);
                let recovery_windows: Vec<WindowId> = std::iter::once(WindowId::PRIMARY)
                    .chain(previous.keys().copied())
                    .collect();
                let mut rebuilt = HashMap::new();
                let mut failed = Vec::new();
                for (id, mut host) in previous {
                    let window = Arc::clone(host.surface.window());
                    host.material = apply_window_surface(
                        window.as_ref(),
                        self.last_theme,
                        host.settings.transparent,
                        self.last_material_mode,
                    );
                    match graphics.create_surface(
                        window,
                        window_wants_transparent_surface(
                            host.settings.transparent,
                            self.last_material_mode,
                        ),
                    ) {
                        Ok(surface) => {
                            let format = surface.format();
                            painters.entry(format).or_insert_with(|| {
                                SceneWgpuPainter::new(
                                    graphics.resources().device(),
                                    graphics.resources().queue(),
                                    format,
                                )
                            });
                            host.surface = surface;
                            rebuilt.insert(id, host);
                        }
                        Err(_) => failed.push((id, host.surface.window().id())),
                    }
                }
                self.graphics = graphics;
                self.painters = painters;
                self.auxiliary = rebuilt;
                self.default_scene_gpu_renderers = Some(default_scene_gpu_renderers_with_host(
                    Arc::clone(self.graphics.resources().device()),
                    Arc::clone(self.graphics.resources().queue()),
                ));
                self.refresh_material();
                self.next_gpu_retry = None;
                self.render_suspended = false;
                invalidate_program_host_textures(recovery_windows, |id| {
                    self.program.host_textures(id)
                });
                self.program.rebuild_gpu(&self.context());
                for (id, window_id) in failed {
                    self.window_ids.remove(&window_id);
                    let update = self
                        .program
                        .window_event(WindowEvent::Closed { id }, &self.context_for(id));
                    self.apply_update(event_loop, update);
                    if event_loop.exiting() {
                        return;
                    }
                }
                self.request_redraw_all();
            }
            Err(_) => {
                self.render_suspended = true;
                self.next_gpu_retry = Some(Instant::now() + GPU_RETRY_INTERVAL);
            }
        }
    }

    fn suspend_rendering(&mut self, _error: HostedGpuError) {
        self.render_suspended = true;
        self.next_gpu_retry = Some(Instant::now() + GPU_RETRY_INTERVAL);
    }

    fn sync_appearance(&mut self) {
        let theme = self.program.theme_mode();
        let mode = self.program.window_material_mode();
        if theme != self.last_theme || mode != self.last_material_mode {
            self.last_theme = theme;
            self.last_material_mode = mode;
            self.refresh_material();
            self.request_redraw_all();
        }
    }

    fn refresh_material(&mut self) {
        clear_system_material(self.graphics.window().as_ref());
        self.material = apply_window_surface(
            self.graphics.window().as_ref(),
            self.last_theme,
            self.settings.transparent,
            self.last_material_mode,
        );
        self.graphics
            .reconfigure_alpha_mode(window_wants_transparent_surface(
                self.settings.transparent,
                self.last_material_mode,
            ));
        for host in self.auxiliary.values_mut() {
            clear_system_material(host.surface.window().as_ref());
            host.material = apply_window_surface(
                host.surface.window().as_ref(),
                self.last_theme,
                host.settings.transparent,
                self.last_material_mode,
            );
            let want_transparent = window_wants_transparent_surface(
                host.settings.transparent,
                self.last_material_mode,
            );
            self.graphics
                .reconfigure_surface_alpha_mode(&mut host.surface, want_transparent);
        }
    }

    fn sync_geometry(&mut self, id: WindowId) {
        if id == WindowId::PRIMARY {
            self.geometry = window_geometry(self.graphics.window());
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            host.geometry = window_geometry(host.surface.window());
        }
        let maximized = self.geometry_of(id).maximized;
        if let Some(session) = self.chrome.get_mut(&id) {
            session.state.update(WindowChromeEvent::MaximizedChanged {
                window: id,
                maximized,
            });
        }
        self.sync_title_bar_maximized(id, maximized);
    }

    fn resize_window(&mut self, id: WindowId) {
        if id == WindowId::PRIMARY {
            self.graphics.resize();
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            self.graphics.resize_surface(&mut host.surface);
        }
    }

    fn sync_window_cursor(&self, id: WindowId) {
        use winit::window::CursorIcon;
        let cursor = self.input_of(id).cursor;
        let icon = self
            .program
            .document(id)
            .and_then(|document| {
                let context = document.context();
                let document_id = document.document();
                context
                    .split_handle_near(document_id, cursor.0, cursor.1)
                    .or_else(|| context.dock_handle_near(document_id, cursor.0, cursor.1))
                    .and_then(|handle| context.world().layout_box(handle))
            })
            .map(|bounds| {
                if bounds.width <= bounds.height {
                    CursorIcon::EwResize
                } else {
                    CursorIcon::NsResize
                }
            })
            .unwrap_or(CursorIcon::Default);
        if let Some(window) = self.window(id) {
            window.set_cursor(icon);
        }
    }

    fn scale_factor(&self, id: WindowId) -> f32 {
        self.window(id)
            .map(|window| normalized_scale_factor(window.scale_factor() as f32))
            .unwrap_or(1.0)
    }

    fn apply_ime_request(&mut self, id: WindowId) {
        let request = resolved_scene_ime_request(self.program.document(id));
        if self.ime_requests.get(&id) == Some(&request) {
            return;
        }
        let Some(window) = self.window(id).cloned() else {
            return;
        };
        // Follow the focused editable field, not NSWindow key status.
        // Gating on has_focus() disables IME while the SCIM candidate panel is
        // key, and also races automation that activates then types immediately.
        apply_text_input_request(window.as_ref(), request);
        self.ime_requests.insert(id, request);
    }

    fn normalized_input(&mut self, id: WindowId, event: &WinitWindowEvent) -> Option<InputEvent> {
        let scale = self.scale_factor(id);
        let origin = self
            .window(id)
            .and_then(|window| window_screen_origin(window));
        self.input_mut(id).map(event, scale, origin)
    }

    fn dispatch_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        input: InputEvent,
    ) -> nana_ui_platform::InputDisposition {
        let disposition = match self
            .program
            .document_mut(id)
            .map(|document| {
                let document_id = document.document();
                RuntimeInputAdapter::default().dispatch_at(
                    document.context_mut(),
                    document_id,
                    &input,
                    self.animation_clock.runtime_time(Instant::now()),
                )
            })
            .transpose()
        {
            Ok(disposition) => disposition.unwrap_or_default(),
            Err(error) => {
                // Drop this input event; the program sees the failure through
                // host_failure instead of the process dying in the event loop.
                self.program.host_failure(HostFailure::InputDispatch {
                    window: id,
                    error: error.to_string(),
                });
                nana_ui_platform::InputDisposition::default()
            }
        };
        let chrome_action = self.title_bar_chrome_action(id, &input);
        // Runtime may already have consumed the event (prevent_default). Scene
        // still delivers input_event so Gallery can drain Activate bindings and
        // Vue can emit JS. Leftover winit handling stays gated by the caller.
        let program_input = self.program.input_event(id, &input, &self.context_for(id));
        if let Err(error) = &program_input {
            self.program.host_failure(HostFailure::InputHandler {
                window: id,
                error: error.to_string(),
            });
        }
        let update = scene_runtime_input_update(disposition, id, program_input);
        let update = self.merge_title_bar_chrome(id, chrome_action, update);
        self.sync_appearance();
        self.apply_update(event_loop, update);
        disposition
    }

    fn title_bar_chrome_action(
        &mut self,
        id: WindowId,
        input: &InputEvent,
    ) -> Option<WindowChromeAction> {
        let program = &self.program;
        let chrome = &mut self.chrome;
        let document = program.document(id)?;
        let session = chrome
            .entry(id)
            .or_insert_with(|| WindowChromeSession::new(id));
        apply_title_bar_pointer(
            &mut session.state,
            &mut session.drag,
            document.context(),
            document.document(),
            input,
        )
    }

    fn merge_title_bar_chrome(
        &mut self,
        id: WindowId,
        action: Option<WindowChromeAction>,
        mut update: RuntimeProgramUpdate,
    ) -> RuntimeProgramUpdate {
        let Some(action) = action else {
            return update;
        };
        if action == WindowChromeAction::Close && id == WindowId::PRIMARY {
            update.exit = true;
            return update;
        }
        let maximized = self
            .chrome
            .get(&id)
            .is_some_and(|session| session.state.is_maximized());
        if action == WindowChromeAction::ToggleMaximize {
            self.sync_title_bar_maximized(id, maximized);
        }
        update
            .window_commands
            .extend(window_commands_for_chrome_action(id, action, maximized));
        update
    }

    fn prepare_window_chrome(&mut self, id: WindowId, maximized: bool) {
        let session = self
            .chrome
            .entry(id)
            .or_insert_with(|| WindowChromeSession::new(id));
        session.state.update(WindowChromeEvent::PrepareWindow(id));
        session.state.update(WindowChromeEvent::MaximizedChanged {
            window: id,
            maximized,
        });
        self.sync_title_bar_maximized(id, maximized);
    }

    fn sync_title_bar_maximized(&mut self, id: WindowId, maximized: bool) {
        let Some(document) = self.program.document_mut(id) else {
            return;
        };
        let document_id = document.document();
        let context = document.context_mut();
        let bars = context
            .world()
            .document_order(document_id)
            .into_iter()
            .filter(|&node| {
                context
                    .read(Entity::<AppTitleBar>::from_stable_id(node), |_| ())
                    .is_ok()
            })
            .collect::<Vec<_>>();
        for bar in bars {
            let _ =
                context.update_component(Entity::<AppTitleBar>::from_stable_id(bar), |bar, _| {
                    bar.maximized = maximized;
                });
        }
    }

    #[cfg(not(target_os = "android"))]
    fn take_accessibility_actions(
        &self,
        id: WindowId,
    ) -> Vec<nana_ui_runtime::AccessibilityActionRequest> {
        if id == WindowId::PRIMARY {
            self.accessibility
                .as_ref()
                .map_or_else(Vec::new, HostedAccessibility::take_actions)
        } else {
            self.auxiliary
                .get(&id)
                .and_then(|host| host.accessibility.as_ref())
                .map_or_else(Vec::new, HostedAccessibility::take_actions)
        }
    }

    #[cfg(not(target_os = "android"))]
    fn synchronize_accessibility(&mut self, id: WindowId) {
        let scale_factor = self.scale_factor(id);
        let has_adapter = if id == WindowId::PRIMARY {
            self.accessibility.is_some()
        } else {
            self.auxiliary
                .get(&id)
                .is_some_and(|host| host.accessibility.is_some())
        };
        if !has_adapter {
            return;
        }
        let scale_factor_changed = if id == WindowId::PRIMARY {
            self.accessibility
                .as_ref()
                .is_some_and(|accessibility| accessibility.scale_factor_changed(scale_factor))
        } else {
            self.auxiliary
                .get(&id)
                .and_then(|host| host.accessibility.as_ref())
                .is_some_and(|accessibility| accessibility.scale_factor_changed(scale_factor))
        };
        let projector_generation = if id == WindowId::PRIMARY {
            self.accessibility
                .as_ref()
                .and_then(HostedAccessibility::retained_generation)
        } else {
            self.auxiliary
                .get(&id)
                .and_then(|host| host.accessibility.as_ref())
                .and_then(HostedAccessibility::retained_generation)
        };
        let pending = self.accessibility_pending_mut(id).take();
        let program = self.program.take_accessibility_update(id);
        let world_generation = accessibility_world_generation(&self.program, id);
        let Some(update) = next_accessibility_update(
            pending,
            program,
            scale_factor_changed,
            projector_generation,
            world_generation,
            || accessibility_snapshot(&self.program, id),
        ) else {
            return;
        };
        if id == WindowId::PRIMARY {
            if let Some(accessibility) = self.accessibility.as_mut() {
                accessibility.synchronize(update, scale_factor);
            }
        } else if let Some(accessibility) = self
            .auxiliary
            .get_mut(&id)
            .and_then(|host| host.accessibility.as_mut())
        {
            accessibility.synchronize(update, scale_factor);
        }
    }

    #[cfg(target_os = "windows")]
    fn take_windows_pen_events(&self, id: WindowId) -> Vec<crate::windows_pen::PenEvent> {
        if id == WindowId::PRIMARY {
            self.pen_hook.drain()
        } else {
            self.auxiliary
                .get(&id)
                .map(|host| host.pen_hook.drain())
                .unwrap_or_default()
        }
    }

    #[cfg(target_os = "windows")]
    fn dispatch_windows_pen_events(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        use crate::windows_pen::PenPhase;

        let events = self.take_windows_pen_events(id);
        let scale = self.scale_factor(id).max(0.01);
        let modifiers = platform_input_modifiers(self.input_of(id).modifiers);
        for event in events {
            let x = event.client_x as f32 / scale;
            let y = event.client_y as f32 / scale;
            self.input_mut(id).cursor = (x, y);
            let input = InputEvent::Pointer {
                phase: match event.phase {
                    PenPhase::Down => PointerPhase::Down,
                    PenPhase::Move => PointerPhase::Move,
                    PenPhase::Up => PointerPhase::Up,
                    PenPhase::Cancel => PointerPhase::Cancel,
                },
                pointer_id: event.pointer_id,
                pointer_type: PointerType::Pen,
                x,
                y,
                screen_x: event.screen_x as f32 / scale,
                screen_y: event.screen_y as f32 / scale,
                button: event.button,
                buttons: event.buttons,
                pressure: event.pressure,
                tangential_pressure: 0.0,
                tilt_x: event.tilt_x,
                tilt_y: event.tilt_y,
                twist: event.twist,
                is_primary: event.is_primary,
                modifiers,
            };
            self.dispatch_input(event_loop, id, input);
            if event_loop.exiting() {
                return;
            }
        }
    }

    fn known_window_ids(&self) -> Vec<WindowId> {
        let mut ids = vec![WindowId::PRIMARY];
        ids.extend(self.auxiliary.keys().copied());
        ids
    }

    fn window(&self, id: WindowId) -> Option<&Arc<winit::window::Window>> {
        if id == WindowId::PRIMARY {
            Some(self.graphics.window())
        } else {
            self.auxiliary.get(&id).map(|host| host.surface.window())
        }
    }

    fn geometry_of(&self, id: WindowId) -> WindowGeometry {
        if id == WindowId::PRIMARY {
            self.geometry
        } else {
            self.auxiliary
                .get(&id)
                .map(|host| host.geometry)
                .unwrap_or_default()
        }
    }

    fn material_of(&self, id: WindowId) -> MaterialOutcome {
        if id == WindowId::PRIMARY {
            self.material
        } else {
            self.auxiliary
                .get(&id)
                .map(|host| host.material)
                .unwrap_or(self.material)
        }
    }

    fn input_of(&self, id: WindowId) -> &InputTracker {
        if id == WindowId::PRIMARY {
            &self.input
        } else {
            self.auxiliary
                .get(&id)
                .map(|host| &host.input)
                .unwrap_or(&self.input)
        }
    }

    fn input_mut(&mut self, id: WindowId) -> &mut InputTracker {
        if id == WindowId::PRIMARY {
            &mut self.input
        } else {
            self.auxiliary
                .get_mut(&id)
                .map(|host| &mut host.input)
                .unwrap_or(&mut self.input)
        }
    }

    fn accessibility_pending_mut(&mut self, id: WindowId) -> &mut Option<AccessibilityUpdate> {
        if id == WindowId::PRIMARY {
            &mut self.accessibility_pending
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            &mut host.accessibility_pending
        } else {
            &mut self.accessibility_pending
        }
    }

    #[cfg(not(target_os = "android"))]
    fn accessibility_mut(&mut self, id: WindowId) -> Option<&mut HostedAccessibility> {
        if id == WindowId::PRIMARY {
            self.accessibility.as_mut()
        } else {
            self.auxiliary
                .get_mut(&id)
                .and_then(|host| host.accessibility.as_mut())
        }
    }

    fn painter_mut(&mut self, format: wgpu::TextureFormat) -> &mut SceneWgpuPainter {
        let resources = self.graphics.resources();
        self.painters
            .entry(format)
            .or_insert_with(|| SceneWgpuPainter::new(resources.device(), resources.queue(), format))
    }

    fn request_redraw(&self, id: WindowId) {
        if let Some(window) = self.window(id) {
            window.request_redraw();
        }
    }

    fn request_redraw_all(&self) {
        for id in self.known_window_ids() {
            self.request_redraw(id);
        }
    }
}

impl<Program: RuntimeProgram> Drop for SceneReady<Program> {
    fn drop(&mut self) {
        clear_system_material(self.graphics.window().as_ref());
    }
}

fn program_context<Message: Send + 'static>(
    proxy: &EventLoopProxy<Message>,
    graphics: &HostedGpuContext,
    id: WindowId,
    geometry: WindowGeometry,
    tasks: SyncSender<Task<Message>>,
    material: MaterialOutcome,
) -> RuntimeProgramContext<Message> {
    let proxy = proxy.clone();
    RuntimeProgramContext::new(
        id,
        geometry,
        graphics.resources(),
        material,
        Arc::new(move |message| {
            let _ = proxy.send_event(message);
        }),
        tasks,
    )
}

fn spawn_task_workers<Message: Send + 'static>(
    proxy: EventLoopProxy<Message>,
) -> SyncSender<Task<Message>> {
    let (sender, receiver) = std::sync::mpsc::sync_channel::<Task<Message>>(TASK_QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    for _ in 0..TASK_WORKERS {
        let receiver = Arc::clone(&receiver);
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            loop {
                let task = {
                    let Ok(receiver) = receiver.lock() else {
                        return;
                    };
                    let Ok(task) = receiver.recv() else {
                        return;
                    };
                    task
                };
                let message = pollster::block_on(task.into_future());
                if proxy.send_event(message).is_err() {
                    return;
                }
            }
        });
    }
    sender
}

fn accessibility_snapshot<Program: RuntimeProgram>(
    program: &Program,
    id: WindowId,
) -> Vec<nana_ui_runtime::AccessibilityNode> {
    program
        .document(id)
        .map(|document| {
            document
                .context()
                .world()
                .project_accessibility(document.document())
        })
        .unwrap_or_default()
}

#[cfg(not(target_os = "android"))]
fn accessibility_world_generation<Program: RuntimeProgram>(
    program: &Program,
    id: WindowId,
) -> Option<u64> {
    program
        .document(id)
        .map(|document| document.context().world().generation())
}

#[cfg(not(target_os = "android"))]
fn next_accessibility_update(
    flush: Option<AccessibilityUpdate>,
    program: Option<AccessibilityUpdate>,
    scale_factor_changed: bool,
    projector_generation: Option<u64>,
    world_generation: Option<u64>,
    snapshot: impl FnOnce() -> Vec<nana_ui_runtime::AccessibilityNode>,
) -> Option<AccessibilityUpdate> {
    if scale_factor_changed {
        return Some(AccessibilityUpdate::Full {
            generation: world_generation,
            nodes: snapshot(),
        });
    }
    if flush.is_some() && program.is_some() {
        return Some(AccessibilityUpdate::Full {
            generation: world_generation,
            nodes: snapshot(),
        });
    }
    if let Some(update) = flush.or(program) {
        let queued = match &update {
            AccessibilityUpdate::Full { generation, .. } => *generation,
            AccessibilityUpdate::Delta(delta) => Some(delta.generation),
        };
        if world_generation.is_some_and(|world| queued.is_some_and(|queued| queued < world)) {
            return Some(AccessibilityUpdate::Full {
                generation: world_generation,
                nodes: snapshot(),
            });
        }
        return Some(update);
    }
    if projector_generation.is_some() && projector_generation == world_generation {
        return None;
    }
    Some(AccessibilityUpdate::Full {
        generation: world_generation,
        nodes: snapshot(),
    })
}

fn window_surface_effect(
    settings_transparent: bool,
    appearance: crate::MaterialEffect,
) -> crate::MaterialEffect {
    if settings_transparent {
        crate::MaterialEffect::Transparent
    } else {
        appearance
    }
}

fn window_wants_transparent_surface(
    settings_transparent: bool,
    appearance: crate::MaterialEffect,
) -> bool {
    window_surface_effect(settings_transparent, appearance).wants_transparent_surface()
}

fn apply_scene_material(
    window: &winit::window::Window,
    theme: crate::ThemeMode,
    requested: crate::MaterialEffect,
) -> MaterialOutcome {
    let appearance = match theme {
        crate::ThemeMode::Dark => Appearance::Dark,
        crate::ThemeMode::Light => Appearance::Light,
    };
    let (red, green, blue, _) = theme.palette().background.to_u8_rgba();
    let alpha = (AppearanceSettings::DEFAULT_BACKDROP_OPACITY * 255.0 + 0.5) as u8;
    apply_hosted_system_material(
        window,
        requested,
        appearance,
        FallbackColor::rgba(red, green, blue, alpha),
    )
}

fn apply_window_transparency(window: &winit::window::Window, requested: crate::MaterialEffect) {
    window.set_transparent(requested.wants_transparent_surface());
}

fn apply_window_surface(
    window: &winit::window::Window,
    theme: crate::ThemeMode,
    settings_transparent: bool,
    appearance: crate::MaterialEffect,
) -> MaterialOutcome {
    let requested = window_surface_effect(settings_transparent, appearance);
    let material = apply_scene_material(window, theme, requested);
    apply_window_transparency(window, requested);
    material
}

fn drag_scene_window(window: &winit::window::Window) {
    if nana_window::drag_custom_title_bar(window) {
        return;
    }
    let _ = window.drag_window();
}

fn scene_paint_viewport(
    geometry: &WindowGeometry,
    material: MaterialOutcome,
    theme: crate::ThemeMode,
) -> ScenePaintViewport {
    ScenePaintViewport {
        logical_size: [geometry.logical_size.0, geometry.logical_size.1],
        physical_size: [geometry.physical_size.0, geometry.physical_size.1],
        scale_factor: geometry.scale_factor,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: scene_clear_color(theme, material),
        clear: true,
    }
}

fn scene_clear_color(theme: crate::ThemeMode, material: MaterialOutcome) -> [f32; 4] {
    if material.effect == MaterialEffect::Transparent {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let color = theme.palette().background;
    let alpha = if material.is_native() {
        AppearanceSettings::DEFAULT_BACKDROP_OPACITY
    } else {
        color.a
    };
    [color.r, color.g, color.b, alpha]
}

fn resolved_scene_ime_request(
    document: Option<&nana_ui_scene::RuntimeDocument>,
) -> TextInputRequest {
    document
        .map(runtime_text_input_request)
        .unwrap_or(TextInputRequest {
            enabled: false,
            cursor_area: None,
            purpose: TextInputPurpose::Normal,
        })
}

fn apply_text_input_request(window: &winit::window::Window, request: TextInputRequest) {
    window.set_ime_allowed(request.enabled);
    if !request.enabled {
        return;
    }
    if let Some(cursor) = request.cursor_area {
        window.set_ime_cursor_area(
            winit::dpi::LogicalPosition::new(cursor.x, cursor.y + cursor.height),
            winit::dpi::LogicalSize::new(cursor.width.max(1.0), cursor.height.max(1.0)),
        );
    }
    window.set_ime_purpose(match request.purpose {
        TextInputPurpose::Normal => winit::window::ImePurpose::Normal,
        TextInputPurpose::Password => winit::window::ImePurpose::Password,
        TextInputPurpose::Terminal => winit::window::ImePurpose::Terminal,
    });
}

fn scene_window_attributes(settings: &RuntimeWindowSettings) -> winit::window::WindowAttributes {
    let mut attributes = winit::window::WindowAttributes::default()
        .with_title(settings.title.clone())
        .with_transparent(settings.transparent)
        .with_resizable(settings.resizable)
        .with_window_level(window_level(settings.always_on_top))
        .with_inner_size(winit::dpi::LogicalSize::new(
            settings.initial_size.0,
            settings.initial_size.1,
        ))
        .with_min_inner_size(winit::dpi::LogicalSize::new(
            settings.minimum_size.0,
            settings.minimum_size.1,
        ))
        .with_maximized(settings.maximized);
    if let Some((x, y)) = settings.initial_position {
        attributes = attributes.with_position(winit::dpi::LogicalPosition::new(x, y));
    }

    apply_scene_window_chrome(attributes, settings)
}

fn apply_scene_window_chrome(
    attributes: winit::window::WindowAttributes,
    settings: &RuntimeWindowSettings,
) -> winit::window::WindowAttributes {
    if settings.system_caption {
        return attributes.with_decorations(true);
    }

    #[cfg(target_os = "macos")]
    {
        attributes
            .with_decorations(true)
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true)
            .with_movable_by_window_background(false)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let attributes = attributes.with_decorations(false);
        #[cfg(target_os = "windows")]
        let attributes = attributes
            .with_undecorated_shadow(true)
            .with_corner_preference(winit::platform::windows::CornerPreference::Round);
        attributes
    }
}

fn scene_aux_window_attributes(
    settings: &RuntimeWindowSettings,
    parent: Option<&winit::window::Window>,
) -> Result<winit::window::WindowAttributes, String> {
    let attributes = scene_window_attributes(settings).with_visible(false);
    #[cfg(target_os = "windows")]
    let attributes = if settings.modal {
        let parent = parent.ok_or_else(|| "modal window requires a parent".to_string())?;
        let handle = parent
            .window_handle()
            .map_err(|error| format!("failed to acquire modal owner handle: {error}"))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("Windows modal owner is not an HWND".into());
        };
        attributes.with_owner_window(handle.hwnd.get())
    } else {
        let _ = parent;
        attributes
    };
    #[cfg(not(target_os = "windows"))]
    let _ = parent;
    Ok(attributes)
}

fn allows_modal_parent_event(event: &WinitWindowEvent) -> bool {
    matches!(
        event,
        WinitWindowEvent::RedrawRequested
            | WinitWindowEvent::Resized(_)
            | WinitWindowEvent::Moved(_)
            | WinitWindowEvent::ScaleFactorChanged { .. }
            | WinitWindowEvent::Occluded(_)
            | WinitWindowEvent::Destroyed
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutedWindowCommand {
    Open(WindowId),
    Focus(WindowId),
    Close(WindowId),
    SetTitle(WindowId),
    Move(WindowId),
    SetBounds(WindowId),
    SetFullscreen(WindowId),
    SetMinimized(WindowId),
    SetMaximized(WindowId),
    SetAlwaysOnTop(WindowId),
    Drag(WindowId),
    Ignore,
}

fn route_window_command(command: &WindowCommand, known: &[WindowId]) -> RoutedWindowCommand {
    let known = |id: WindowId| known.contains(&id);
    match command {
        WindowCommand::Open { id, .. } if known(*id) => RoutedWindowCommand::Focus(*id),
        WindowCommand::Open { id, .. } => RoutedWindowCommand::Open(*id),
        WindowCommand::Close(id) if *id == WindowId::PRIMARY || !known(*id) => {
            RoutedWindowCommand::Ignore
        }
        WindowCommand::Close(id) => RoutedWindowCommand::Close(*id),
        WindowCommand::Focus(id) if known(*id) => RoutedWindowCommand::Focus(*id),
        WindowCommand::SetTitle { id, .. } if known(*id) => RoutedWindowCommand::SetTitle(*id),
        WindowCommand::Move { id, .. } if known(*id) => RoutedWindowCommand::Move(*id),
        WindowCommand::SetBounds { id, .. } if known(*id) => RoutedWindowCommand::SetBounds(*id),
        WindowCommand::SetFullscreen { id, .. } if known(*id) => {
            RoutedWindowCommand::SetFullscreen(*id)
        }
        WindowCommand::SetMinimized { id, .. } if known(*id) => {
            RoutedWindowCommand::SetMinimized(*id)
        }
        WindowCommand::SetMaximized { id, .. } if known(*id) => {
            RoutedWindowCommand::SetMaximized(*id)
        }
        WindowCommand::SetAlwaysOnTop { id, .. } if known(*id) => {
            RoutedWindowCommand::SetAlwaysOnTop(*id)
        }
        WindowCommand::Drag(id) if known(*id) => RoutedWindowCommand::Drag(*id),
        _ => RoutedWindowCommand::Ignore,
    }
}

fn windows_to_redraw(redraw: RuntimeRedraw, known: &[WindowId]) -> Vec<WindowId> {
    match redraw {
        RuntimeRedraw::None => Vec::new(),
        RuntimeRedraw::Window(id) => known.iter().copied().filter(|known| *known == id).collect(),
        RuntimeRedraw::All => known.to_vec(),
    }
}

/// Drop HostTexture views bound to the previous Device, then the caller runs
/// `rebuild_gpu` so programs can re-register on the new one.
fn invalidate_program_host_textures(
    window_ids: impl IntoIterator<Item = WindowId>,
    mut host_textures: impl FnMut(WindowId) -> Option<HostTextureRegistry>,
) -> usize {
    let mut invalidated = 0;
    for id in window_ids {
        if let Some(registry) = host_textures(id) {
            invalidated += registry.invalidate_all();
        }
    }
    invalidated
}

fn should_deliver_program_ime(modal_blocks: bool) -> bool {
    !modal_blocks
}

/// Always invoke the program input hook. Runtime `prevent_default` still
/// requests a window redraw; it does not drop Gallery/Vue delivery. A failed
/// handler degrades to an empty update (the caller reports it via
/// `host_failure`) instead of panicking.
fn scene_runtime_input_update(
    disposition: nana_ui_platform::InputDisposition,
    id: WindowId,
    program_input: Result<RuntimeProgramUpdate, FrameworkError>,
) -> RuntimeProgramUpdate {
    let program_update = program_input.unwrap_or_default();
    if disposition.prevent_default {
        RuntimeProgramUpdate::redraw(id).merge(program_update)
    } else {
        program_update
    }
}

fn window_level(always_on_top: bool) -> winit::window::WindowLevel {
    if always_on_top {
        winit::window::WindowLevel::AlwaysOnTop
    } else {
        winit::window::WindowLevel::Normal
    }
}

fn window_geometry(window: &winit::window::Window) -> WindowGeometry {
    let scale_factor = normalized_scale_factor(window.scale_factor() as f32);
    let physical_size = window.inner_size();
    let physical_position = window.outer_position().ok();
    WindowGeometry {
        physical_position: physical_position.map(|position| (position.x, position.y)),
        physical_size: (physical_size.width, physical_size.height),
        logical_position: physical_position.map(|position| {
            let logical = position.to_logical::<f32>(f64::from(scale_factor));
            (logical.x, logical.y)
        }),
        logical_size: (
            physical_size.width as f32 / scale_factor,
            physical_size.height as f32 / scale_factor,
        ),
        scale_factor,
        maximized: window.is_maximized(),
    }
}

fn window_screen_origin(window: &winit::window::Window) -> Option<(f32, f32)> {
    let scale = window.scale_factor().max(0.01);
    window.outer_position().ok().map(|position| {
        let origin = position.to_logical::<f32>(scale);
        (origin.x, origin.y)
    })
}

fn normalized_scale_factor(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

fn platform_input_key(key: &winit::keyboard::Key) -> Option<String> {
    Some(match key {
        winit::keyboard::Key::Named(named) => format!("{named:?}"),
        winit::keyboard::Key::Character(character) => character.to_string(),
        winit::keyboard::Key::Unidentified(_) | winit::keyboard::Key::Dead(_) => return None,
    })
}

fn platform_input_modifiers(value: ModifiersState) -> InputModifiers {
    InputModifiers {
        alt: value.alt_key(),
        control: value.control_key(),
        meta: value.super_key(),
        shift: value.shift_key(),
    }
}

fn platform_ime_event(ime: winit::event::Ime) -> ImeEvent {
    match ime {
        winit::event::Ime::Enabled => ImeEvent::Enabled,
        winit::event::Ime::Disabled => ImeEvent::Disabled,
        winit::event::Ime::Preedit(text, selection) => ImeEvent::Preedit { text, selection },
        winit::event::Ime::Commit(text) => ImeEvent::Commit(text),
    }
}

fn mouse_button_code(button: MouseButton) -> i16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(button) => button.min(i16::MAX as u16) as i16,
    }
}

fn mouse_button_mask(button: i16) -> u16 {
    match button {
        0 => 1,
        1 => 4,
        2 => 2,
        3 => 8,
        4 => 16,
        _ => 0,
    }
}

fn screen_position(origin: Option<(f32, f32)>, client: (f32, f32)) -> (f32, f32) {
    origin.map_or(client, |origin| (origin.0 + client.0, origin.1 + client.1))
}

#[derive(Debug, Default)]
struct InputTracker {
    cursor: (f32, f32),
    buttons: u16,
    modifiers: ModifiersState,
    active_touches: HashSet<u64>,
    primary_touch: Option<u64>,
    pending_file_paths: Vec<PathBuf>,
    file_drop_emitted: bool,
}

impl InputTracker {
    fn clear_pointers(&mut self) {
        self.buttons = 0;
        self.active_touches.clear();
        self.primary_touch = None;
    }

    fn map(
        &mut self,
        event: &WinitWindowEvent,
        scale: f32,
        screen_origin: Option<(f32, f32)>,
    ) -> Option<InputEvent> {
        let modifiers = platform_input_modifiers(self.modifiers);
        match event {
            WinitWindowEvent::CursorMoved { position, .. } => {
                let point = position.to_logical::<f32>(f64::from(scale));
                self.cursor = (point.x, point.y);
                let screen = screen_position(screen_origin, self.cursor);
                Some(InputEvent::Pointer {
                    phase: PointerPhase::Move,
                    pointer_id: 1,
                    pointer_type: PointerType::Mouse,
                    x: self.cursor.0,
                    y: self.cursor.1,
                    screen_x: screen.0,
                    screen_y: screen.1,
                    button: -1,
                    buttons: self.buttons,
                    pressure: if self.buttons == 0 { 0.0 } else { 0.5 },
                    tangential_pressure: 0.0,
                    tilt_x: 0,
                    tilt_y: 0,
                    twist: 0,
                    is_primary: true,
                    modifiers,
                })
            }
            WinitWindowEvent::MouseInput { state, button, .. } => {
                let button = mouse_button_code(*button);
                let pressed = *state == ElementState::Pressed;
                let mask = mouse_button_mask(button);
                if pressed {
                    self.buttons |= mask;
                } else {
                    self.buttons &= !mask;
                }
                let screen = screen_position(screen_origin, self.cursor);
                Some(InputEvent::Pointer {
                    phase: if pressed {
                        PointerPhase::Down
                    } else {
                        PointerPhase::Up
                    },
                    pointer_id: 1,
                    pointer_type: PointerType::Mouse,
                    x: self.cursor.0,
                    y: self.cursor.1,
                    screen_x: screen.0,
                    screen_y: screen.1,
                    button,
                    buttons: self.buttons,
                    pressure: if self.buttons == 0 { 0.0 } else { 0.5 },
                    tangential_pressure: 0.0,
                    tilt_x: 0,
                    tilt_y: 0,
                    twist: 0,
                    is_primary: true,
                    modifiers,
                })
            }
            WinitWindowEvent::CursorLeft { .. } => {
                let screen = screen_position(screen_origin, self.cursor);
                let buttons = std::mem::take(&mut self.buttons);
                Some(InputEvent::Pointer {
                    phase: PointerPhase::Cancel,
                    pointer_id: 1,
                    pointer_type: PointerType::Mouse,
                    x: self.cursor.0,
                    y: self.cursor.1,
                    screen_x: screen.0,
                    screen_y: screen.1,
                    button: -1,
                    buttons,
                    pressure: 0.0,
                    tangential_pressure: 0.0,
                    tilt_x: 0,
                    tilt_y: 0,
                    twist: 0,
                    is_primary: true,
                    modifiers,
                })
            }
            WinitWindowEvent::Touch(touch) => {
                let point = touch.location.to_logical::<f32>(f64::from(scale));
                let client = (point.x, point.y);
                let screen = screen_position(screen_origin, client);
                let phase = match touch.phase {
                    TouchPhase::Started => PointerPhase::Down,
                    TouchPhase::Moved => PointerPhase::Move,
                    TouchPhase::Ended => PointerPhase::Up,
                    TouchPhase::Cancelled => PointerPhase::Cancel,
                };
                if matches!(touch.phase, TouchPhase::Started) {
                    if self.active_touches.is_empty() {
                        self.primary_touch = Some(touch.id);
                    }
                    self.active_touches.insert(touch.id);
                }
                if matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    self.active_touches.remove(&touch.id);
                }
                let is_primary = self.primary_touch == Some(touch.id);
                if self.active_touches.is_empty() {
                    self.active_touches.clear();
                    self.primary_touch = None;
                }
                Some(InputEvent::Pointer {
                    phase,
                    pointer_id: touch.id.saturating_add(2),
                    pointer_type: PointerType::Touch,
                    x: client.0,
                    y: client.1,
                    screen_x: screen.0,
                    screen_y: screen.1,
                    button: 0,
                    buttons: if matches!(phase, PointerPhase::Down | PointerPhase::Move) {
                        1
                    } else {
                        0
                    },
                    pressure: if matches!(phase, PointerPhase::Down | PointerPhase::Move) {
                        touch
                            .force
                            .as_ref()
                            .map(|force| force.normalized() as f32)
                            .unwrap_or(0.5)
                            .clamp(0.0, 1.0)
                    } else {
                        0.0
                    },
                    tangential_pressure: 0.0,
                    tilt_x: 0,
                    tilt_y: 0,
                    twist: 0,
                    is_primary,
                    modifiers,
                })
            }
            WinitWindowEvent::MouseWheel { delta, .. } => {
                let (delta_x, delta_y, line_delta) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x, *y, true),
                    MouseScrollDelta::PixelDelta(delta) => (
                        (delta.x / f64::from(scale)) as f32,
                        (delta.y / f64::from(scale)) as f32,
                        false,
                    ),
                };
                Some(InputEvent::Wheel {
                    x: self.cursor.0,
                    y: self.cursor.1,
                    delta_x,
                    delta_y,
                    line_delta,
                    modifiers,
                })
            }
            WinitWindowEvent::KeyboardInput { event, .. } => Some(InputEvent::Keyboard {
                pressed: event.state == ElementState::Pressed,
                key: platform_input_key(&event.logical_key).unwrap_or_default(),
                text: event.text.as_ref().map(ToString::to_string),
                code: format!("{:?}", event.physical_key),
                repeat: event.repeat,
                modifiers,
            }),
            _ => None,
        }
    }

    fn map_file_window_event(
        &mut self,
        event: &WinitWindowEvent,
        id: WindowId,
    ) -> Option<WindowEvent> {
        match event {
            WinitWindowEvent::HoveredFile(path) => {
                self.file_drop_emitted = false;
                self.pending_file_paths.push(path.clone());
                Some(WindowEvent::FileHovered {
                    id,
                    paths: self.pending_file_paths.clone(),
                    position: Some(self.cursor),
                })
            }
            WinitWindowEvent::HoveredFileCancelled => {
                self.pending_file_paths.clear();
                self.file_drop_emitted = false;
                Some(WindowEvent::FileHoverCancelled { id })
            }
            WinitWindowEvent::DroppedFile(path) => {
                if self.file_drop_emitted {
                    return None;
                }
                self.file_drop_emitted = true;
                let paths = if self.pending_file_paths.is_empty() {
                    vec![path.clone()]
                } else {
                    std::mem::take(&mut self.pending_file_paths)
                };
                Some(WindowEvent::FileDropped {
                    id,
                    paths,
                    position: Some(self.cursor),
                })
            }
            _ => None,
        }
    }
}

fn platform_window_event(
    event: &WinitWindowEvent,
    id: WindowId,
    geometry: WindowGeometry,
) -> Option<WindowEvent> {
    Some(match event {
        WinitWindowEvent::CloseRequested => WindowEvent::CloseRequested { id },
        WinitWindowEvent::Destroyed => WindowEvent::Closed { id },
        WinitWindowEvent::Occluded(hidden) => WindowEvent::VisibilityChanged {
            id,
            hidden: *hidden,
        },
        WinitWindowEvent::Focused(focused) => WindowEvent::FocusChanged {
            id,
            focused: *focused,
        },
        WinitWindowEvent::Ime(ime) => WindowEvent::Ime {
            id,
            event: platform_ime_event(ime.clone()),
        },
        WinitWindowEvent::Resized(_) | WinitWindowEvent::ScaleFactorChanged { .. } => {
            WindowEvent::Resized { id, geometry }
        }
        WinitWindowEvent::Moved(_) => WindowEvent::Moved { id, geometry },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "android"))]
    use super::next_accessibility_update;
    use super::{
        InputTracker, RoutedWindowCommand, invalidate_program_host_textures, mouse_button_code,
        mouse_button_mask, platform_ime_event, platform_input_key, platform_input_modifiers,
        platform_window_event, resolved_scene_ime_request, route_window_command,
        scene_runtime_input_update, scene_window_attributes, screen_position,
        should_deliver_program_ime, window_level, window_surface_effect,
        window_wants_transparent_surface, windows_to_redraw,
    };
    use crate::{
        HostTexture, HostTextureAlphaMode, HostTextureRegistry, MaterialEffect,
        RuntimeProgramUpdate, RuntimeRedraw,
    };
    use nana_ui_platform::{
        ImeEvent, InputDisposition, InputEvent, PointerPhase, PointerType, TextInputPurpose,
        WindowCommand, WindowEvent, WindowGeometry, WindowId, WindowSettings,
    };
    #[cfg(not(target_os = "android"))]
    use nana_ui_runtime::{AccessibilityDelta, AccessibilityUpdate, FrameworkError};
    use std::path::PathBuf;
    use winit::dpi::PhysicalPosition;
    use winit::event::{
        DeviceId, ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase,
        WindowEvent as WinitWindowEvent,
    };
    use winit::keyboard::{Key, ModifiersState, NamedKey};

    fn geometry() -> WindowGeometry {
        WindowGeometry {
            physical_size: (200, 100),
            logical_size: (100.0, 50.0),
            scale_factor: 2.0,
            ..WindowGeometry::default()
        }
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn empty_flush_reprojects_when_the_program_already_drained_runtime_work() {
        let Some(AccessibilityUpdate::Full { generation, nodes }) =
            next_accessibility_update(None, None, false, None, Some(3), Vec::new)
        else {
            panic!("drained SystemWork must still reach AccessKit from the world");
        };
        assert_eq!(generation, Some(3));
        assert!(nodes.is_empty());
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn matching_generations_do_not_rebuild_an_idle_tree() {
        assert!(
            next_accessibility_update(None, None, false, Some(3), Some(3), || panic!(
                "idle frames must not snapshot"
            ))
            .is_none()
        );
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn program_accessibility_queue_is_the_host_source_when_flush_is_empty() {
        let queued = AccessibilityUpdate::Delta(AccessibilityDelta {
            generation: 2,
            updated: Vec::new(),
            removed: Vec::new(),
        });
        assert_eq!(
            next_accessibility_update(None, Some(queued.clone()), false, None, Some(2), || panic!(
                "queued deltas must not force a world snapshot"
            ),),
            Some(queued)
        );
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn scale_change_reprojects_even_when_generations_match() {
        let Some(AccessibilityUpdate::Full { generation, .. }) =
            next_accessibility_update(None, None, true, Some(1), Some(1), Vec::new)
        else {
            panic!("DPI change must reproject the current world");
        };
        assert_eq!(generation, Some(1));
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn flush_and_program_deltas_reproject_the_world_instead_of_dropping_one() {
        let flush = AccessibilityUpdate::Delta(AccessibilityDelta {
            generation: 3,
            updated: Vec::new(),
            removed: Vec::new(),
        });
        let program = AccessibilityUpdate::Delta(AccessibilityDelta {
            generation: 2,
            updated: Vec::new(),
            removed: Vec::new(),
        });
        let Some(AccessibilityUpdate::Full { generation, .. }) = next_accessibility_update(
            Some(flush),
            Some(program),
            false,
            Some(1),
            Some(3),
            Vec::new,
        ) else {
            panic!("two accessibility sources must not drop hierarchy for layout");
        };
        assert_eq!(generation, Some(3));
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn stale_program_queue_yields_to_the_current_world_snapshot() {
        let queued = AccessibilityUpdate::Delta(AccessibilityDelta {
            generation: 1,
            updated: Vec::new(),
            removed: Vec::new(),
        });
        let Some(AccessibilityUpdate::Full { generation, .. }) =
            next_accessibility_update(None, Some(queued), false, Some(1), Some(3), Vec::new)
        else {
            panic!("stale queued delta must reproject AccessKit from the world");
        };
        assert_eq!(generation, Some(3));
    }

    #[test]
    fn scene_windows_use_client_chrome_and_runtime_settings() {
        let mut settings = WindowSettings::new("Scene");
        settings.transparent = true;
        settings.always_on_top = true;
        settings.resizable = false;
        settings.maximized = true;
        settings.initial_size = (640.0, 480.0);
        settings.minimum_size = (320.0, 240.0);
        let attributes = scene_window_attributes(&settings);

        assert_eq!(attributes.title, "Scene");
        #[cfg(target_os = "macos")]
        assert!(attributes.decorations);
        #[cfg(not(target_os = "macos"))]
        assert!(!attributes.decorations);
        assert!(attributes.transparent);
        assert!(attributes.maximized);
        assert!(!attributes.resizable);
        assert_eq!(
            attributes.window_level,
            winit::window::WindowLevel::AlwaysOnTop
        );
        assert_eq!(window_level(false), winit::window::WindowLevel::Normal);

        settings.system_caption = true;
        let caption = scene_window_attributes(&settings);
        assert!(caption.decorations);
    }

    #[test]
    fn transparent_aux_keeps_transparent_when_primary_appearance_is_solid() {
        let appearance = MaterialEffect::Solid;
        assert_eq!(
            window_surface_effect(false, appearance),
            MaterialEffect::Solid
        );
        assert_eq!(
            window_surface_effect(true, appearance),
            MaterialEffect::Transparent
        );
        assert!(window_surface_effect(true, appearance).wants_transparent_surface());
        assert!(window_wants_transparent_surface(true, appearance));
        assert!(!window_wants_transparent_surface(false, appearance));
    }

    #[test]
    fn transparent_surface_picks_non_opaque_alpha_before_surface_lock() {
        let appearance = MaterialEffect::Solid;
        let transparent = window_wants_transparent_surface(true, appearance);
        assert!(transparent);
        assert_eq!(
            crate::hosted_context::preferred_alpha_mode(
                &[
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::CompositeAlphaMode::PreMultiplied,
                    wgpu::CompositeAlphaMode::PostMultiplied,
                ],
                transparent,
            ),
            wgpu::CompositeAlphaMode::PreMultiplied
        );
        assert_eq!(
            crate::hosted_context::preferred_alpha_mode(
                &[
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::CompositeAlphaMode::PostMultiplied,
                ],
                transparent,
            ),
            wgpu::CompositeAlphaMode::PostMultiplied
        );
        let opaque = window_wants_transparent_surface(false, appearance);
        assert!(!opaque);
        assert_eq!(
            crate::hosted_context::preferred_alpha_mode(
                &[
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::CompositeAlphaMode::PostMultiplied,
                ],
                opaque,
            ),
            wgpu::CompositeAlphaMode::Opaque
        );
    }

    #[test]
    fn input_key_uses_named_debug_and_character_text() {
        assert_eq!(
            platform_input_key(&Key::Named(NamedKey::ArrowDown)),
            Some("ArrowDown".into())
        );
        assert_eq!(
            platform_input_key(&Key::Character("V".into())),
            Some("V".into())
        );
        assert!(
            platform_input_key(&Key::Unidentified(winit::keyboard::NativeKey::Unidentified))
                .is_none()
        );
    }

    #[test]
    fn modifiers_map_control_alt_shift_and_meta() {
        let modifiers = platform_input_modifiers(
            ModifiersState::CONTROL
                | ModifiersState::ALT
                | ModifiersState::SHIFT
                | ModifiersState::SUPER,
        );
        assert!(modifiers.control);
        assert!(modifiers.alt);
        assert!(modifiers.shift);
        assert!(modifiers.meta);
    }

    #[test]
    fn mouse_buttons_use_the_hosted_mask_contract() {
        assert_eq!(mouse_button_code(MouseButton::Left), 0);
        assert_eq!(mouse_button_code(MouseButton::Right), 2);
        assert_eq!(mouse_button_mask(0), 1);
        assert_eq!(mouse_button_mask(1), 4);
        assert_eq!(mouse_button_mask(2), 2);
    }

    #[test]
    fn pointer_move_down_and_leave_match_hosted_coordinates() {
        let mut tracker = InputTracker::default();
        let moved = tracker
            .map(
                &WinitWindowEvent::CursorMoved {
                    device_id: DeviceId::dummy(),
                    position: PhysicalPosition::new(20.0, 40.0),
                },
                2.0,
                Some((100.0, 200.0)),
            )
            .expect("cursor move");
        let InputEvent::Pointer {
            phase,
            x,
            y,
            screen_x,
            screen_y,
            pointer_type,
            button,
            ..
        } = moved
        else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Move);
        assert_eq!(pointer_type, PointerType::Mouse);
        assert_eq!((x, y), (10.0, 20.0));
        assert_eq!((screen_x, screen_y), (110.0, 220.0));
        assert_eq!(button, -1);

        let down = tracker
            .map(
                &WinitWindowEvent::MouseInput {
                    device_id: DeviceId::dummy(),
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                },
                2.0,
                Some((100.0, 200.0)),
            )
            .expect("mouse down");
        let InputEvent::Pointer {
            phase,
            buttons,
            pressure,
            ..
        } = down
        else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Down);
        assert_eq!(buttons, 1);
        assert_eq!(pressure, 0.5);

        let left = tracker
            .map(
                &WinitWindowEvent::CursorLeft {
                    device_id: DeviceId::dummy(),
                },
                2.0,
                Some((100.0, 200.0)),
            )
            .expect("cursor left");
        let InputEvent::Pointer {
            phase, x, buttons, ..
        } = left
        else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Cancel);
        assert_eq!(x, 10.0);
        assert_eq!(buttons, 1);
    }

    #[test]
    fn wheel_preserves_line_delta_and_converts_pixels() {
        let mut tracker = InputTracker {
            cursor: (8.0, 16.0),
            ..InputTracker::default()
        };
        let line = tracker
            .map(
                &WinitWindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta: MouseScrollDelta::LineDelta(1.0, -2.0),
                    phase: TouchPhase::Moved,
                },
                2.0,
                None,
            )
            .expect("line wheel");
        assert_eq!(
            line,
            InputEvent::Wheel {
                x: 8.0,
                y: 16.0,
                delta_x: 1.0,
                delta_y: -2.0,
                line_delta: true,
                modifiers: Default::default(),
            }
        );

        let pixel = tracker
            .map(
                &WinitWindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(8.0, -4.0)),
                    phase: TouchPhase::Moved,
                },
                2.0,
                None,
            )
            .expect("pixel wheel");
        let InputEvent::Wheel {
            delta_x,
            delta_y,
            line_delta,
            ..
        } = pixel
        else {
            panic!("expected wheel");
        };
        assert_eq!((delta_x, delta_y), (4.0, -2.0));
        assert!(!line_delta);
    }

    #[test]
    fn first_touch_is_primary_and_uses_hosted_pointer_id() {
        let mut tracker = InputTracker::default();
        let start = tracker
            .map(
                &WinitWindowEvent::Touch(Touch {
                    device_id: DeviceId::dummy(),
                    phase: TouchPhase::Started,
                    location: PhysicalPosition::new(4.0, 8.0),
                    force: None,
                    id: 3,
                }),
                2.0,
                None,
            )
            .expect("touch start");
        let InputEvent::Pointer {
            phase,
            pointer_id,
            pointer_type,
            is_primary,
            x,
            y,
            buttons,
            ..
        } = start
        else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Down);
        assert_eq!(pointer_id, 5);
        assert_eq!(pointer_type, PointerType::Touch);
        assert!(is_primary);
        assert_eq!((x, y), (2.0, 4.0));
        assert_eq!(buttons, 1);
    }

    #[test]
    fn ime_and_window_lifecycle_mapping_is_backend_neutral() {
        assert_eq!(
            platform_ime_event(winit::event::Ime::Commit("你".into())),
            ImeEvent::Commit("你".into())
        );
        assert_eq!(
            platform_ime_event(winit::event::Ime::Preedit("かな".into(), Some((3, 6)))),
            ImeEvent::Preedit {
                text: "かな".into(),
                selection: Some((3, 6)),
            }
        );
        assert_eq!(
            platform_window_event(
                &WinitWindowEvent::CloseRequested,
                WindowId::PRIMARY,
                geometry(),
            ),
            Some(WindowEvent::CloseRequested {
                id: WindowId::PRIMARY
            })
        );
        assert_eq!(
            platform_window_event(
                &WinitWindowEvent::Focused(true),
                WindowId::PRIMARY,
                geometry(),
            ),
            Some(WindowEvent::FocusChanged {
                id: WindowId::PRIMARY,
                focused: true,
            })
        );
        assert_eq!(
            platform_window_event(
                &WinitWindowEvent::Occluded(true),
                WindowId::PRIMARY,
                geometry(),
            ),
            Some(WindowEvent::VisibilityChanged {
                id: WindowId::PRIMARY,
                hidden: true,
            })
        );
        assert_eq!(
            platform_window_event(
                &WinitWindowEvent::Ime(winit::event::Ime::Enabled),
                WindowId::PRIMARY,
                geometry(),
            ),
            Some(WindowEvent::Ime {
                id: WindowId::PRIMARY,
                event: ImeEvent::Enabled,
            })
        );
        assert!(
            platform_window_event(
                &WinitWindowEvent::RedrawRequested,
                WindowId::PRIMARY,
                geometry(),
            )
            .is_none()
        );
    }

    #[test]
    fn file_drag_batches_hover_paths_and_emits_one_drop() {
        let mut tracker = InputTracker {
            cursor: (24.0, 48.0),
            ..InputTracker::default()
        };
        let first = PathBuf::from("a.png");
        let second = PathBuf::from("b.png");
        assert_eq!(
            tracker
                .map_file_window_event(
                    &WinitWindowEvent::HoveredFile(first.clone()),
                    WindowId::PRIMARY,
                )
                .and_then(|event| match event {
                    WindowEvent::FileHovered {
                        paths, position, ..
                    } => Some((paths, position)),
                    _ => None,
                }),
            Some((vec![first.clone()], Some((24.0, 48.0))))
        );
        tracker.map_file_window_event(
            &WinitWindowEvent::HoveredFile(second.clone()),
            WindowId::PRIMARY,
        );
        assert_eq!(
            tracker
                .map_file_window_event(
                    &WinitWindowEvent::DroppedFile(first.clone()),
                    WindowId::PRIMARY,
                )
                .and_then(|event| match event {
                    WindowEvent::FileDropped { paths, .. } => Some(paths),
                    _ => None,
                }),
            Some(vec![first, second.clone()])
        );
        assert!(
            tracker
                .map_file_window_event(&WinitWindowEvent::DroppedFile(second), WindowId::PRIMARY,)
                .is_none()
        );

        let mut cancelled = InputTracker::default();
        cancelled.map_file_window_event(
            &WinitWindowEvent::HoveredFile(PathBuf::from("gone.png")),
            WindowId::PRIMARY,
        );
        assert!(matches!(
            cancelled
                .map_file_window_event(&WinitWindowEvent::HoveredFileCancelled, WindowId::PRIMARY,),
            Some(WindowEvent::FileHoverCancelled { .. })
        ));
        let solo = PathBuf::from("solo.txt");
        assert_eq!(
            cancelled
                .map_file_window_event(
                    &WinitWindowEvent::DroppedFile(solo.clone()),
                    WindowId::PRIMARY,
                )
                .and_then(|event| match event {
                    WindowEvent::FileDropped { paths, .. } => Some(paths),
                    _ => None,
                }),
            Some(vec![solo])
        );
    }

    #[test]
    fn scene_ime_follows_focused_text_input_without_window_key_status() {
        let disabled = resolved_scene_ime_request(None);
        assert!(!disabled.enabled);
        assert_eq!(disabled.purpose, TextInputPurpose::Normal);

        let document_id = nana_ui_runtime::DocumentId::new(1).unwrap();
        let mut document = nana_ui_scene::RuntimeDocument::new(document_id);
        let input = document
            .context_mut()
            .create_component(document_id, nana_ui_runtime::TextInput::new("NanaUI"))
            .unwrap();
        assert!(!resolved_scene_ime_request(Some(&document)).enabled);

        assert!(
            document
                .context_mut()
                .focus_node(document_id, input.stable_id())
                .unwrap()
        );
        let enabled = resolved_scene_ime_request(Some(&document));
        assert!(enabled.enabled);
        assert_eq!(enabled.purpose, TextInputPurpose::Normal);
    }

    #[test]
    fn screen_position_falls_back_to_client_without_origin() {
        assert_eq!(screen_position(None, (3.0, 4.0)), (3.0, 4.0));
        assert_eq!(
            screen_position(Some((10.0, 20.0)), (3.0, 4.0)),
            (13.0, 24.0)
        );
    }

    #[test]
    fn window_commands_route_by_known_ids_without_a_surface() {
        let primary = WindowId::PRIMARY;
        let tool = WindowId(7);
        let known = [primary, tool];
        let settings = WindowSettings::new("tool");

        assert_eq!(
            route_window_command(
                &WindowCommand::Open {
                    id: tool,
                    settings: settings.clone(),
                },
                &known
            ),
            RoutedWindowCommand::Focus(tool)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::Open {
                    id: WindowId(9),
                    settings,
                },
                &known
            ),
            RoutedWindowCommand::Open(WindowId(9))
        );
        assert_eq!(
            route_window_command(&WindowCommand::Close(primary), &known),
            RoutedWindowCommand::Ignore
        );
        assert_eq!(
            route_window_command(&WindowCommand::Close(tool), &known),
            RoutedWindowCommand::Close(tool)
        );
        assert_eq!(
            route_window_command(&WindowCommand::Close(WindowId(3)), &known),
            RoutedWindowCommand::Ignore
        );
        assert_eq!(
            route_window_command(&WindowCommand::Focus(tool), &known),
            RoutedWindowCommand::Focus(tool)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::SetTitle {
                    id: tool,
                    title: "Aux".into(),
                },
                &known
            ),
            RoutedWindowCommand::SetTitle(tool)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::Move {
                    id: tool,
                    position: (8.0, 16.0),
                },
                &known
            ),
            RoutedWindowCommand::Move(tool)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::SetBounds {
                    id: primary,
                    position: (0.0, 0.0),
                    size: (100.0, 80.0),
                },
                &known
            ),
            RoutedWindowCommand::SetBounds(primary)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::SetFullscreen {
                    id: WindowId(3),
                    fullscreen: true,
                },
                &known
            ),
            RoutedWindowCommand::Ignore
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::SetMinimized {
                    id: tool,
                    minimized: true,
                },
                &known
            ),
            RoutedWindowCommand::SetMinimized(tool)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::SetMaximized {
                    id: tool,
                    maximized: true,
                },
                &known
            ),
            RoutedWindowCommand::SetMaximized(tool)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::SetAlwaysOnTop {
                    id: tool,
                    always_on_top: true,
                },
                &known
            ),
            RoutedWindowCommand::SetAlwaysOnTop(tool)
        );
        assert_eq!(
            route_window_command(&WindowCommand::Drag(tool), &known),
            RoutedWindowCommand::Drag(tool)
        );
        assert_eq!(
            route_window_command(&WindowCommand::Drag(WindowId(3)), &known),
            RoutedWindowCommand::Ignore
        );
    }

    #[test]
    fn redraw_requests_ignore_unknown_ids_and_cover_every_known_window() {
        let primary = WindowId::PRIMARY;
        let tool = WindowId(2);
        let known = [primary, tool];

        assert!(windows_to_redraw(RuntimeRedraw::None, &known).is_empty());
        assert_eq!(
            windows_to_redraw(RuntimeRedraw::Window(tool), &known),
            vec![tool]
        );
        assert!(windows_to_redraw(RuntimeRedraw::Window(WindowId(9)), &known).is_empty());
        assert_eq!(
            windows_to_redraw(RuntimeRedraw::All, &known),
            vec![primary, tool]
        );
    }

    #[test]
    fn runtime_ime_ownership_does_not_drop_program_notification() {
        assert!(should_deliver_program_ime(false));
        assert!(!should_deliver_program_ime(true));
    }

    #[test]
    fn runtime_prevent_default_still_invokes_the_program_input_hook() {
        let update = scene_runtime_input_update(
            InputDisposition {
                prevent_default: true,
            },
            WindowId::PRIMARY,
            Ok(RuntimeProgramUpdate::exit()),
        );
        assert!(update.exit);
        assert_eq!(update.redraw, RuntimeRedraw::Window(WindowId::PRIMARY));

        let update = scene_runtime_input_update(
            InputDisposition {
                prevent_default: false,
            },
            WindowId::PRIMARY,
            Ok(RuntimeProgramUpdate::default()),
        );
        assert_eq!(update.redraw, RuntimeRedraw::None);
    }

    #[test]
    fn failed_program_input_degrades_without_panicking() {
        let update = scene_runtime_input_update(
            InputDisposition {
                prevent_default: true,
            },
            WindowId::PRIMARY,
            Err(FrameworkError::InvalidAction),
        );
        // The failed handler's effect is dropped, but the Runtime's
        // prevent_default redraw still happens instead of a panic.
        assert_eq!(update.redraw, RuntimeRedraw::Window(WindowId::PRIMARY));
        assert!(!update.exit);
        assert!(update.window_commands.is_empty());
    }

    #[test]
    fn device_recovery_invalidates_cloned_program_host_textures_before_rebuild() {
        let registry = occupied_host_textures("live");
        struct FakeProgram {
            textures: HostTextureRegistry,
            rebuilt_len: std::cell::Cell<Option<usize>>,
        }
        impl FakeProgram {
            fn host_textures(&self, id: WindowId) -> Option<HostTextureRegistry> {
                match id {
                    WindowId::PRIMARY | WindowId(2) => Some(self.textures.clone()),
                    _ => None,
                }
            }

            fn rebuild_gpu(&self) {
                self.rebuilt_len.set(Some(self.textures.len()));
            }
        }

        let program = FakeProgram {
            textures: registry.clone(),
            rebuilt_len: std::cell::Cell::new(None),
        };
        let cleared =
            invalidate_program_host_textures([WindowId::PRIMARY, WindowId(2), WindowId(9)], |id| {
                program.host_textures(id)
            });
        program.rebuild_gpu();

        assert_eq!(cleared, 1);
        assert_eq!(program.rebuilt_len.get(), Some(0));
        assert!(registry.is_empty());
        assert_eq!(
            invalidate_program_host_textures([WindowId::PRIMARY, WindowId(2)], |id| program
                .host_textures(id)),
            0
        );
    }

    fn occupied_host_textures(slot: &str) -> HostTextureRegistry {
        let (device, _) = test_device();
        let registry = HostTextureRegistry::new();
        registry.register(
            slot,
            HostTexture::from_wgpu(1, 1, test_texture_view(&device)),
            8,
            8,
            HostTextureAlphaMode::Premultiplied,
        );
        registry
    }

    fn test_texture_view(device: &wgpu::Device) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("NanaUI scene host recovery test texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn test_device() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .expect("scene host recovery test requires a WGPU adapter");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("NanaUI scene host recovery test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("scene host recovery test requires a WGPU device")
    }
}
