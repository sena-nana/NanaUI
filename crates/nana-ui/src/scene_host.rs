//! Nana-owned winit + wgpu loop for [`crate::RuntimeProgram`].
//!
//! Applications never see Iced Message/Element/window IDs. Paint goes through
//! [`crate::SceneWgpuPainter`].

use std::collections::HashSet;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced_wgpu::wgpu;
use iced_winit::futures::futures::executor;
use nana_ui_platform::{
    ImeEvent, InputEvent, InputModifiers, PointerPhase, PointerType, TextInputPurpose,
    TextInputRequest, WindowCommand, WindowEvent, WindowGeometry, WindowId,
};
use nana_ui_runtime::{AccessibilityUpdate, LayoutViewport, Task};
use nana_window::{
    Appearance, FallbackColor, MaterialEffect, MaterialOutcome, apply_hosted_system_material,
    clear_system_material,
};
use winit::application::ApplicationHandler;
use winit::event::{
    ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent as WinitWindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;

#[cfg(not(target_os = "android"))]
use crate::accessibility::HostedAccessibility;
use crate::nana_text::NanaTextShaper;
use crate::runtime_host::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw,
    RuntimeWindowSettings, gated_runtime_input_update, gated_runtime_window_update,
    runtime_text_input_request,
};
use crate::scene_paint::{ScenePaintViewport, SceneWgpuPainter};
use crate::theme::ThemeModeExt;
use crate::{
    HostedGpuContext, HostedGpuError, HostedRunError, HostedSurfaceFrame, RuntimeAnimationClock,
    RuntimeInputAdapter, SceneGpuRendererRegistry, default_scene_gpu_renderers_with_host,
    resolve_scene_gpu_renderers,
};

const GPU_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const TASK_QUEUE_CAPACITY: usize = 256;
const TASK_WORKERS: usize = 4;

/// Run a [`RuntimeProgram`] on the Nana Scene host (no Iced UserInterface).
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

struct SceneReady<Program: RuntimeProgram> {
    program: Program,
    graphics: HostedGpuContext,
    painter: SceneWgpuPainter,
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
    #[cfg(target_os = "windows")]
    pen_hook: crate::windows_pen::WindowsPenHook,
    next_gpu_retry: Option<Instant>,
    render_suspended: bool,
    last_theme: crate::ThemeMode,
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
        _window_id: winit::window::WindowId,
        event: WinitWindowEvent,
    ) {
        let Self::Ready(ready) = self else {
            return;
        };
        ready.handle_window_event(event_loop, event);
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
    let graphics = executor::block_on(HostedGpuContext::new(
        Arc::clone(&window),
        wgpu::Features::empty(),
    ))
    .map_err(|error| error.to_string())?;
    let painter = SceneWgpuPainter::new(
        graphics.resources().device(),
        graphics.resources().queue(),
        graphics.format(),
    );
    let tasks = spawn_task_workers(proxy.clone());
    let geometry = window_geometry(graphics.window());
    let context = program_context(&proxy, &graphics, geometry, tasks.clone());
    let (program, startup) = Program::initialize(&context).map_err(|error| error.to_string())?;
    let default_scene_gpu_renderers = Some(default_scene_gpu_renderers_with_host(
        Arc::clone(graphics.resources().device()),
        Arc::clone(graphics.resources().queue()),
    ));
    let last_theme = program.theme_mode();
    let material = apply_scene_material(graphics.window().as_ref(), last_theme);
    #[cfg(not(target_os = "android"))]
    let accessibility = {
        let nodes = accessibility_snapshot(&program);
        Some(HostedAccessibility::new(
            event_loop,
            Arc::clone(graphics.window()),
            None,
            nodes,
            true,
            window.scale_factor() as f32,
        ))
    };
    let mut ready = SceneReady {
        program,
        graphics,
        painter,
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
        #[cfg(target_os = "windows")]
        pen_hook: crate::windows_pen::WindowsPenHook::install(window.as_ref())?,
        next_gpu_retry: None,
        render_suspended: false,
        last_theme,
    };
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
        program_context(
            &self.proxy,
            &self.graphics,
            self.geometry,
            self.tasks.clone(),
        )
    }

    fn process_message(&mut self, event_loop: &ActiveEventLoop, message: Program::Message) {
        let update = self.program.update(message, &self.context());
        self.sync_theme();
        self.apply_update(event_loop, update);
    }

    fn handle_window_event(&mut self, event_loop: &ActiveEventLoop, event: WinitWindowEvent) {
        #[cfg(not(target_os = "android"))]
        if let Some(accessibility) = self.accessibility.as_mut() {
            accessibility.process_event(self.graphics.window().as_ref(), &event);
        }
        #[cfg(not(target_os = "android"))]
        for request in self.take_accessibility_actions() {
            let update = self
                .program
                .accessibility_action(WindowId::PRIMARY, request, &self.context())
                .unwrap_or_else(|error| {
                    panic!("RuntimeProgram accessibility action failed: {error}")
                });
            self.apply_update(event_loop, update);
            if event_loop.exiting() {
                return;
            }
        }
        #[cfg(target_os = "windows")]
        self.dispatch_windows_pen_events(event_loop);
        if let WinitWindowEvent::ModifiersChanged(modifiers) = &event {
            self.input.modifiers = modifiers.state();
        }
        if let WinitWindowEvent::CursorMoved { position, .. } = &event {
            let scale = self.scale_factor();
            let point = position.to_logical::<f32>(f64::from(scale));
            self.input.cursor = (point.x, point.y);
            self.sync_window_cursor();
        }
        if let Some(input) = self.normalized_input(&event) {
            let disposition = self.dispatch_input(event_loop, input);
            if disposition.prevent_default || event_loop.exiting() {
                return;
            }
        }
        match &event {
            WinitWindowEvent::RedrawRequested => self.redraw(event_loop),
            WinitWindowEvent::CloseRequested => {
                if let Some(window_event) =
                    platform_window_event(&event, WindowId::PRIMARY, self.geometry)
                {
                    let update = self.program.window_event(window_event, &self.context());
                    self.apply_update(event_loop, update);
                }
                event_loop.exit();
            }
            WinitWindowEvent::Destroyed => {
                if let Some(window_event) =
                    platform_window_event(&event, WindowId::PRIMARY, self.geometry)
                {
                    let update = self.program.window_event(window_event, &self.context());
                    self.apply_update(event_loop, update);
                }
                event_loop.exit();
            }
            WinitWindowEvent::Moved(_) => {
                self.sync_geometry();
                if let Some(window_event) =
                    platform_window_event(&event, WindowId::PRIMARY, self.geometry)
                {
                    let update = self.program.window_event(window_event, &self.context());
                    self.apply_update(event_loop, update);
                }
            }
            WinitWindowEvent::Resized(_) | WinitWindowEvent::ScaleFactorChanged { .. } => {
                self.graphics.resize();
                self.sync_geometry();
                if let Some(window_event) =
                    platform_window_event(&event, WindowId::PRIMARY, self.geometry)
                {
                    let update = self.program.window_event(window_event, &self.context());
                    self.apply_update(event_loop, update);
                }
                self.graphics.window().request_redraw();
            }
            WinitWindowEvent::Occluded(_) => {
                if let Some(window_event) =
                    platform_window_event(&event, WindowId::PRIMARY, self.geometry)
                {
                    let update = self.program.window_event(window_event, &self.context());
                    self.apply_update(event_loop, update);
                }
            }
            WinitWindowEvent::Focused(focused) => {
                if !*focused {
                    self.input.clear_pointers();
                }
                if let Some(window_event) =
                    platform_window_event(&event, WindowId::PRIMARY, self.geometry)
                {
                    let update = self.program.window_event(window_event, &self.context());
                    self.apply_update(event_loop, update);
                }
                self.apply_ime_request();
            }
            WinitWindowEvent::Ime(ime) => {
                self.handle_ime(event_loop, platform_ime_event(ime.clone()))
            }
            _ => {}
        }
    }

    fn handle_ime(&mut self, event_loop: &ActiveEventLoop, event: ImeEvent) {
        let window_event = WindowEvent::Ime {
            id: WindowId::PRIMARY,
            event: event.clone(),
        };
        let mut runtime_ime_owned = false;
        let ime_changed = self
            .program
            .document_mut(WindowId::PRIMARY)
            .map(|document| {
                runtime_ime_owned = true;
                let document_id = document.document();
                RuntimeInputAdapter::default()
                    .dispatch_ime(document.context_mut(), document_id, &event)
                    .map(|disposition| {
                        disposition.prevent_default && !matches!(event, ImeEvent::Enabled)
                    })
            })
            .transpose()
            .unwrap_or_else(|error| panic!("RuntimeProgram IME dispatch failed: {error}"))
            .unwrap_or(false);
        let modal_blocks_ime = self
            .program
            .document(WindowId::PRIMARY)
            .is_some_and(|document| {
                document
                    .context()
                    .has_blocking_runtime_overlay(document.document())
            });
        let mut update = gated_runtime_window_update(runtime_ime_owned || modal_blocks_ime, || {
            self.program.window_event(window_event, &self.context())
        });
        if ime_changed {
            update = update.merge(RuntimeProgramUpdate::redraw(WindowId::PRIMARY));
        }
        self.sync_theme();
        self.apply_update(event_loop, update);
        self.apply_ime_request();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.graphics.take_device_lost()
            || self.next_gpu_retry.is_some_and(|deadline| now >= deadline)
        {
            self.recover_device();
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
        self.program
            .document(WindowId::PRIMARY)
            .and_then(|document| self.animation_clock.next_wakeup(document.context()))
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
        let frame = self
            .program
            .document_mut(WindowId::PRIMARY)
            .map(|document| self.animation_clock.wake(document.context_mut(), now));
        if let Some(frame) = frame {
            let had_samples = frame.has_updates();
            update = update.merge(
                self.program
                    .animation_frame(WindowId::PRIMARY, frame, &self.context())
                    .unwrap_or_else(|error| {
                        panic!("RuntimeProgram animation handler failed: {error}")
                    }),
            );
            if had_samples {
                update = update.merge(RuntimeProgramUpdate::redraw(WindowId::PRIMARY));
            }
        }
        self.sync_theme();
        self.apply_update(event_loop, update);
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        if self.render_suspended {
            return;
        }
        if self.graphics.take_device_lost() {
            self.recover_device();
            return;
        }
        self.program
            .prepare_window_frame(WindowId::PRIMARY, &self.context());
        let viewport =
            LayoutViewport::new(self.geometry.logical_size.0, self.geometry.logical_size.1);
        let scene = {
            let document = self
                .program
                .document_mut(WindowId::PRIMARY)
                .unwrap_or_else(|| {
                    panic!(
                        "RuntimeProgram has no document for window {}",
                        WindowId::PRIMARY.0
                    )
                });
            let update = document
                .flush(viewport, &mut self.text)
                .unwrap_or_else(|error| panic!("RuntimeProgram frame did not settle: {error}"));
            if !update.accessibility.updated.is_empty() || !update.accessibility.removed.is_empty()
            {
                self.accessibility_pending = Some(AccessibilityUpdate::Delta(update.accessibility));
            }
            document.shared_scene()
        };
        if let Some(producers) = self.program.scene_resource_producers(WindowId::PRIMARY) {
            producers
                .encode_scene(
                    scene.as_ref(),
                    self.graphics.resources().device(),
                    self.graphics.resources().queue(),
                )
                .unwrap_or_else(|error| {
                    panic!("RuntimeProgram resource production failed: {error}")
                });
        }
        let frame = match self.graphics.acquire_frame() {
            Ok(HostedSurfaceFrame::Ready(frame)) => frame,
            Ok(HostedSurfaceFrame::Retry) => {
                self.graphics.window().request_redraw();
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
        let host_textures = self.program.host_textures(WindowId::PRIMARY);
        let gpu_renderers = resolve_scene_gpu_renderers(
            self.program.scene_gpu_renderers(WindowId::PRIMARY),
            self.default_scene_gpu_renderers.clone(),
        );
        self.painter
            .paint(
                scene.as_ref(),
                &mut encoder,
                &target,
                scene_paint_viewport(&self.geometry, self.material, self.program.theme_mode()),
                host_textures.as_ref(),
                gpu_renderers.as_ref(),
            )
            .unwrap_or_else(|error| {
                panic!("RuntimeProgram produced an unpaintable UiScene: {error}")
            });
        self.graphics.resources().queue().submit([encoder.finish()]);
        self.graphics.present(frame);
        self.apply_ime_request();
        let update = self
            .program
            .window_frame_presented(WindowId::PRIMARY, &self.context());
        self.sync_theme();
        self.apply_update(event_loop, update);
        #[cfg(not(target_os = "android"))]
        self.synchronize_accessibility();
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
        match update.redraw {
            RuntimeRedraw::None => {}
            RuntimeRedraw::Window(id) if id != WindowId::PRIMARY => {}
            RuntimeRedraw::Window(_) | RuntimeRedraw::All => {
                self.graphics.window().request_redraw();
            }
        }
    }

    fn apply_window_command(&mut self, event_loop: &ActiveEventLoop, command: WindowCommand) {
        let window = Arc::clone(self.graphics.window());
        match command {
            WindowCommand::Open { .. } => {}
            WindowCommand::Close(id) if id == WindowId::PRIMARY => {}
            WindowCommand::Close(_) => {}
            WindowCommand::Focus(id) if id == WindowId::PRIMARY => {
                window.set_visible(true);
                window.focus_window();
            }
            WindowCommand::SetTitle { id, title } if id == WindowId::PRIMARY => {
                window.set_title(&title);
            }
            WindowCommand::Move { id, position } if id == WindowId::PRIMARY => {
                window.set_outer_position(winit::dpi::Position::Logical(
                    winit::dpi::LogicalPosition::new(f64::from(position.0), f64::from(position.1)),
                ));
            }
            WindowCommand::SetBounds { id, position, size } if id == WindowId::PRIMARY => {
                window.set_outer_position(winit::dpi::Position::Logical(
                    winit::dpi::LogicalPosition::new(f64::from(position.0), f64::from(position.1)),
                ));
                let _ = window.request_inner_size(winit::dpi::Size::Logical(
                    winit::dpi::LogicalSize::new(
                        f64::from(size.0.max(1.0)),
                        f64::from(size.1.max(1.0)),
                    ),
                ));
            }
            WindowCommand::SetFullscreen { id, fullscreen } if id == WindowId::PRIMARY => {
                window.set_fullscreen(
                    fullscreen.then_some(winit::window::Fullscreen::Borderless(None)),
                );
            }
            WindowCommand::SetMinimized { id, minimized } if id == WindowId::PRIMARY => {
                window.set_minimized(minimized);
            }
            WindowCommand::SetMaximized { id, maximized } if id == WindowId::PRIMARY => {
                window.set_maximized(maximized);
                self.graphics.resize();
                self.sync_geometry();
                let update = self.program.window_event(
                    WindowEvent::Resized {
                        id: WindowId::PRIMARY,
                        geometry: self.geometry,
                    },
                    &self.context(),
                );
                self.apply_update(event_loop, update);
            }
            WindowCommand::SetAlwaysOnTop { id, always_on_top } if id == WindowId::PRIMARY => {
                window.set_window_level(window_level(always_on_top));
            }
            WindowCommand::Focus(_)
            | WindowCommand::SetTitle { .. }
            | WindowCommand::Move { .. }
            | WindowCommand::SetBounds { .. }
            | WindowCommand::SetFullscreen { .. }
            | WindowCommand::SetMinimized { .. }
            | WindowCommand::SetMaximized { .. }
            | WindowCommand::SetAlwaysOnTop { .. } => {}
        }
    }

    fn recover_device(&mut self) {
        let window = Arc::clone(self.graphics.window());
        match executor::block_on(HostedGpuContext::new(window, wgpu::Features::empty())) {
            Ok(graphics) => {
                self.painter = SceneWgpuPainter::new(
                    graphics.resources().device(),
                    graphics.resources().queue(),
                    graphics.format(),
                );
                self.default_scene_gpu_renderers = Some(default_scene_gpu_renderers_with_host(
                    Arc::clone(graphics.resources().device()),
                    Arc::clone(graphics.resources().queue()),
                ));
                self.graphics = graphics;
                self.refresh_material();
                self.next_gpu_retry = None;
                self.render_suspended = false;
                self.program.rebuild_gpu(&self.context());
                self.graphics.window().request_redraw();
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

    fn sync_theme(&mut self) {
        let theme = self.program.theme_mode();
        if theme != self.last_theme {
            self.last_theme = theme;
            self.refresh_material();
            self.graphics.window().request_redraw();
        }
    }

    fn refresh_material(&mut self) {
        clear_system_material(self.graphics.window().as_ref());
        self.material = apply_scene_material(self.graphics.window().as_ref(), self.last_theme);
    }

    fn sync_geometry(&mut self) {
        self.geometry = window_geometry(self.graphics.window());
    }

    fn sync_window_cursor(&self) {
        use winit::window::CursorIcon;
        let icon = self
            .program
            .document(WindowId::PRIMARY)
            .and_then(|document| {
                let context = document.context();
                let id = document.document();
                let (x, y) = self.input.cursor;
                context
                    .split_handle_near(id, x, y)
                    .or_else(|| context.dock_handle_near(id, x, y))
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
        self.graphics.window().set_cursor(icon);
    }

    fn scale_factor(&self) -> f32 {
        normalized_scale_factor(self.graphics.window().scale_factor() as f32)
    }

    fn apply_ime_request(&self) {
        apply_text_input_request(
            self.graphics.window().as_ref(),
            self.program
                .document(WindowId::PRIMARY)
                .map(runtime_text_input_request),
        );
    }

    fn normalized_input(&mut self, event: &WinitWindowEvent) -> Option<InputEvent> {
        self.input.map(
            event,
            self.scale_factor(),
            window_screen_origin(self.graphics.window()),
        )
    }

    fn dispatch_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        input: InputEvent,
    ) -> nana_ui_platform::InputDisposition {
        let disposition = self
            .program
            .document_mut(WindowId::PRIMARY)
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
            .unwrap_or_else(|error| panic!("RuntimeProgram input dispatch failed: {error}"))
            .unwrap_or_default();
        let update = gated_runtime_input_update(disposition, WindowId::PRIMARY, || {
            self.program
                .input_event(WindowId::PRIMARY, &input, &self.context())
        });
        self.sync_theme();
        self.apply_update(event_loop, update);
        disposition
    }

    #[cfg(not(target_os = "android"))]
    fn take_accessibility_actions(&self) -> Vec<nana_ui_runtime::AccessibilityActionRequest> {
        self.accessibility
            .as_ref()
            .map_or_else(Vec::new, HostedAccessibility::take_actions)
    }

    #[cfg(not(target_os = "android"))]
    fn synchronize_accessibility(&mut self) {
        let scale_factor = self.scale_factor();
        let Some(accessibility) = self.accessibility.as_mut() else {
            return;
        };
        let scale_factor_changed = accessibility.scale_factor_changed(scale_factor);
        let pending = self.accessibility_pending.take();
        let update = if scale_factor_changed {
            match pending {
                Some(update @ AccessibilityUpdate::Full { .. }) => Some(update),
                Some(AccessibilityUpdate::Delta(delta)) => Some(AccessibilityUpdate::Full {
                    generation: Some(delta.generation),
                    nodes: accessibility_snapshot(&self.program),
                }),
                None => Some(AccessibilityUpdate::Full {
                    generation: None,
                    nodes: accessibility_snapshot(&self.program),
                }),
            }
        } else {
            pending
        };
        if let Some(update) = update {
            accessibility.synchronize(update, scale_factor);
        }
    }

    #[cfg(target_os = "windows")]
    fn dispatch_windows_pen_events(&mut self, event_loop: &ActiveEventLoop) {
        use crate::windows_pen::PenPhase;

        let scale = self.scale_factor().max(0.01);
        let modifiers = platform_input_modifiers(self.input.modifiers);
        for event in self.pen_hook.drain() {
            let x = event.client_x as f32 / scale;
            let y = event.client_y as f32 / scale;
            self.input.cursor = (x, y);
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
            self.dispatch_input(event_loop, input);
            if event_loop.exiting() {
                return;
            }
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
    geometry: WindowGeometry,
    tasks: SyncSender<Task<Message>>,
) -> RuntimeProgramContext<Message> {
    let proxy = proxy.clone();
    RuntimeProgramContext::new(
        WindowId::PRIMARY,
        geometry,
        graphics.resources(),
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
                let message = executor::block_on(task.into_future());
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
) -> Vec<nana_ui_runtime::AccessibilityNode> {
    program
        .document(WindowId::PRIMARY)
        .map(|document| {
            document
                .context()
                .world()
                .project_accessibility(document.document())
        })
        .unwrap_or_default()
}

fn apply_scene_material(
    window: &winit::window::Window,
    theme: crate::ThemeMode,
) -> MaterialOutcome {
    let (appearance, fallback) = match theme {
        crate::ThemeMode::Dark => (Appearance::Dark, FallbackColor::rgba(24, 24, 24, 220)),
        crate::ThemeMode::Light => (Appearance::Light, FallbackColor::rgba(255, 255, 255, 232)),
    };
    apply_hosted_system_material(window, appearance, fallback)
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
    let color = theme.colors().background;
    let alpha = if material.is_native() { 0.78 } else { color.a };
    [color.r, color.g, color.b, alpha]
}

fn apply_text_input_request(window: &winit::window::Window, request: Option<TextInputRequest>) {
    let Some(request) = request else {
        return;
    };
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

    attributes.with_decorations(true)
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
    use super::{
        InputTracker, mouse_button_code, mouse_button_mask, platform_ime_event, platform_input_key,
        platform_input_modifiers, platform_window_event, scene_window_attributes, screen_position,
        window_level,
    };
    use nana_ui_platform::{
        ImeEvent, InputEvent, PointerPhase, PointerType, WindowEvent, WindowGeometry, WindowId,
        WindowSettings,
    };
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

    #[test]
    fn scene_windows_use_native_chrome_and_runtime_settings() {
        let mut settings = WindowSettings::new("Scene");
        settings.transparent = true;
        settings.always_on_top = true;
        settings.resizable = false;
        settings.maximized = true;
        settings.initial_size = (640.0, 480.0);
        settings.minimum_size = (320.0, 240.0);
        let attributes = scene_window_attributes(&settings);

        assert_eq!(attributes.title, "Scene");
        assert!(attributes.decorations);
        assert!(attributes.transparent);
        assert!(attributes.maximized);
        assert!(!attributes.resizable);
        assert_eq!(
            attributes.window_level,
            winit::window::WindowLevel::AlwaysOnTop
        );
        assert_eq!(window_level(false), winit::window::WindowLevel::Normal);
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
    fn screen_position_falls_back_to_client_without_origin() {
        assert_eq!(screen_position(None, (3.0, 4.0)), (3.0, 4.0));
        assert_eq!(
            screen_position(Some((10.0, 20.0)), (3.0, 4.0)),
            (13.0, 24.0)
        );
    }
}
