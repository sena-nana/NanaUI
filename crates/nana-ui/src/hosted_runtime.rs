//! High-level native runtime for applications that host NanaUI in one WGPU context.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::{Element, Point, Size};
use iced_wgpu::graphics::core::renderer;
use iced_wgpu::wgpu;
use iced_winit::futures::futures::executor;
use iced_winit::winit;
#[cfg(target_os = "macos")]
use nana_window::MaterialFallback;
#[cfg(not(target_os = "macos"))]
use nana_window::{Appearance, FallbackColor, apply_system_material};
use nana_window::{MaterialOutcome, clear_system_material, prepare_custom_title_bar};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};

use crate::{
    HostedGpuContext, HostedGpuError, HostedGpuResources, HostedGpuSurface, HostedSurfaceFrame,
    HostedUiRenderer, HostedUiTarget, ThemeMode, WindowChromeAction,
};

/// Stable application-owned identity for one hosted window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostedWindowId(pub u64);

impl HostedWindowId {
    pub const PRIMARY: Self = Self(0);
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HostedWindowRole {
    #[default]
    Main,
    Tool,
}

/// Native window configuration for [`run_hosted`] and auxiliary windows.
#[derive(Debug, Clone)]
pub struct HostedWindowSettings {
    pub title: String,
    pub initial_size: Size<f64>,
    pub minimum_size: Size<f64>,
    pub initial_position: Option<(f64, f64)>,
    pub initial_physical_geometry: Option<(i32, i32, u32, u32)>,
    pub maximized: bool,
    pub transparent: bool,
    pub role: HostedWindowRole,
    pub gpu_retry_interval: Duration,
}

impl HostedWindowSettings {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            initial_size: Size::new(1200.0, 800.0),
            minimum_size: Size::new(760.0, 520.0),
            initial_position: None,
            initial_physical_geometry: None,
            maximized: false,
            transparent: !cfg!(target_os = "macos"),
            role: HostedWindowRole::Main,
            gpu_retry_interval: Duration::from_secs(2),
        }
    }

    pub fn initial_size(mut self, width: f64, height: f64) -> Self {
        self.initial_size = Size::new(width, height);
        self.initial_physical_geometry = None;
        self
    }

    pub fn minimum_size(mut self, width: f64, height: f64) -> Self {
        self.minimum_size = Size::new(width, height);
        self
    }

    pub fn initial_position(mut self, x: f64, y: f64) -> Self {
        self.initial_position = Some((x, y));
        self.initial_physical_geometry = None;
        self
    }

    pub fn physical_geometry(mut self, x: i32, y: i32, width: u32, height: u32) -> Self {
        self.initial_physical_geometry = Some((x, y, width, height));
        self
    }

    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    pub fn transparent(mut self, transparent: bool) -> Self {
        self.transparent = transparent;
        self
    }

    pub fn role(mut self, role: HostedWindowRole) -> Self {
        self.role = role;
        self
    }

    pub fn tool_window(mut self) -> Self {
        self.role = HostedWindowRole::Tool;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostedWindowGeometry {
    pub physical_position: Option<(i32, i32)>,
    pub physical_size: Size<u32>,
    pub logical_position: Option<Point>,
    pub logical_size: Size,
    pub scale_factor: f32,
    pub maximized: bool,
}

/// Stable application-facing state for the primary native host.
#[derive(Clone)]
pub struct HostedProgramContext<Message: 'static> {
    proxy: EventLoopProxy<Message>,
    gpu: HostedGpuResources,
    window_id: iced::window::Id,
    geometry: HostedWindowGeometry,
    window_hidden: bool,
    drawable: bool,
    surface_format: wgpu::TextureFormat,
}

impl<Message: 'static> HostedProgramContext<Message> {
    pub fn proxy(&self) -> &EventLoopProxy<Message> {
        &self.proxy
    }

    pub fn gpu(&self) -> &HostedGpuResources {
        &self.gpu
    }

    pub const fn window_id(&self) -> iced::window::Id {
        self.window_id
    }

    pub const fn geometry(&self) -> HostedWindowGeometry {
        self.geometry
    }

    pub const fn physical_size(&self) -> Size<u32> {
        self.geometry.physical_size
    }

    pub const fn logical_size(&self) -> Size {
        self.geometry.logical_size
    }

    pub const fn scale_factor(&self) -> f32 {
        self.geometry.scale_factor
    }

    pub const fn maximized(&self) -> bool {
        self.geometry.maximized
    }

    pub const fn window_hidden(&self) -> bool {
        self.window_hidden
    }

    pub const fn drawable(&self) -> bool {
        self.drawable
    }

    pub const fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }
}

/// Window lifecycle information translated into logical and physical geometry.
#[derive(Debug, Clone, Copy)]
pub enum HostedWindowEvent {
    Ready {
        id: HostedWindowId,
        window_id: iced::window::Id,
        geometry: HostedWindowGeometry,
    },
    Resized {
        id: HostedWindowId,
        window_id: iced::window::Id,
        geometry: HostedWindowGeometry,
    },
    Moved {
        id: HostedWindowId,
        window_id: iced::window::Id,
        geometry: HostedWindowGeometry,
    },
    VisibilityChanged {
        id: HostedWindowId,
        window_id: iced::window::Id,
        hidden: bool,
    },
    CloseRequested {
        id: HostedWindowId,
        window_id: iced::window::Id,
    },
}

impl HostedWindowEvent {
    pub const fn id(self) -> HostedWindowId {
        match self {
            Self::Ready { id, .. }
            | Self::Resized { id, .. }
            | Self::Moved { id, .. }
            | Self::VisibilityChanged { id, .. }
            | Self::CloseRequested { id, .. } => id,
        }
    }
}

/// Typed host failures and recoveries. User-facing copy remains app-owned.
#[derive(Debug, Clone)]
pub enum HostedRuntimeEvent {
    RenderingSuspended(HostedGpuError),
    DeviceRecovered,
    DeviceRecoveryFailed(HostedGpuError),
    WindowOpenFailed { id: HostedWindowId, message: String },
}

#[derive(Debug, Clone)]
pub enum HostedWindowCommand {
    Open {
        id: HostedWindowId,
        settings: HostedWindowSettings,
    },
    Close(HostedWindowId),
    Focus(HostedWindowId),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HostedRedraw {
    #[default]
    None,
    Primary,
    Window(HostedWindowId),
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostedWindowAction {
    pub id: HostedWindowId,
    pub action: WindowChromeAction,
}

/// Commands returned after a program handles one event.
#[derive(Debug, Clone, Default)]
pub struct HostedProgramUpdate {
    pub window_action: Option<HostedWindowAction>,
    pub redraw: HostedRedraw,
    pub window_commands: Vec<HostedWindowCommand>,
    pub exit: bool,
}

impl HostedProgramUpdate {
    pub const fn redraw() -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::Primary,
            window_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn redraw_window(id: HostedWindowId) -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::Window(id),
            window_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn redraw_all() -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::All,
            window_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn exit() -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::None,
            window_commands: Vec::new(),
            exit: true,
        }
    }

    pub const fn with_window_action(action: WindowChromeAction) -> Self {
        Self::with_target_window_action(HostedWindowId::PRIMARY, action)
    }

    pub const fn with_target_window_action(id: HostedWindowId, action: WindowChromeAction) -> Self {
        Self {
            window_action: Some(HostedWindowAction { id, action }),
            redraw: HostedRedraw::Window(id),
            window_commands: Vec::new(),
            exit: false,
        }
    }

    pub fn with_window_commands(
        mut self,
        commands: impl IntoIterator<Item = HostedWindowCommand>,
    ) -> Self {
        self.window_commands.extend(commands);
        self
    }
}

/// Business application contract driven by the NanaUI hosted runtime.
pub trait HostedProgram: Sized + 'static {
    type Message: Send + 'static;
    type Error: fmt::Display;

    fn initialize(
        context: &HostedProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error>;

    fn update(
        &mut self,
        message: Self::Message,
        context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate;

    fn view(&self, native_material: bool) -> Element<'_, Self::Message>;

    fn view_window(&self, id: HostedWindowId, native_material: bool) -> Element<'_, Self::Message> {
        let _ = id;
        self.view(native_material)
    }

    fn theme_mode(&self) -> ThemeMode;

    fn window_event(
        &mut self,
        _event: HostedWindowEvent,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        HostedProgramUpdate::default()
    }

    fn runtime_event(
        &mut self,
        _event: HostedRuntimeEvent,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        HostedProgramUpdate::default()
    }

    fn next_wakeup(&self) -> Option<Instant> {
        None
    }

    fn wake(
        &mut self,
        _now: Instant,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        HostedProgramUpdate::default()
    }

    fn prepare_frame(&mut self, _context: &HostedProgramContext<Self::Message>) {}

    fn prepare_window_frame(
        &mut self,
        id: HostedWindowId,
        context: &HostedProgramContext<Self::Message>,
    ) {
        if id == HostedWindowId::PRIMARY {
            self.prepare_frame(context);
        }
    }

    fn rebuild_gpu(&mut self, _context: &HostedProgramContext<Self::Message>) {}

    fn frame_presented(
        &mut self,
        _material: MaterialOutcome,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        HostedProgramUpdate::default()
    }

    fn window_frame_presented(
        &mut self,
        id: HostedWindowId,
        material: MaterialOutcome,
        context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        if id == HostedWindowId::PRIMARY {
            self.frame_presented(material, context)
        } else {
            HostedProgramUpdate::default()
        }
    }
}

pub fn run_hosted<Program: HostedProgram>(
    settings: HostedWindowSettings,
) -> Result<(), HostedRunError> {
    let event_loop = EventLoop::<Program::Message>::with_user_event()
        .build()
        .map_err(HostedRunError::EventLoop)?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut runner = HostedRunner::<Program>::Loading {
        proxy: event_loop.create_proxy(),
        settings,
        failure: None,
    };
    event_loop
        .run_app(&mut runner)
        .map_err(HostedRunError::EventLoop)?;
    runner.into_result()
}

enum HostedRunner<Program: HostedProgram> {
    Loading {
        proxy: EventLoopProxy<Program::Message>,
        settings: HostedWindowSettings,
        failure: Option<String>,
    },
    Ready(Box<HostedReady<Program>>),
    Finished {
        failure: Option<String>,
    },
}

struct HostedAuxiliary {
    surface: HostedGpuSurface,
    ui: HostedUiRenderer,
    iced_window_id: iced::window::Id,
    material: MaterialOutcome,
}

impl Drop for HostedAuxiliary {
    fn drop(&mut self) {
        clear_system_material(self.surface.window().as_ref());
    }
}

struct HostedReady<Program: HostedProgram> {
    graphics: HostedGpuContext,
    ui: HostedUiRenderer,
    program: Program,
    proxy: EventLoopProxy<Program::Message>,
    iced_window_id: iced::window::Id,
    material: MaterialOutcome,
    settings: HostedWindowSettings,
    auxiliary: HashMap<HostedWindowId, HostedAuxiliary>,
    window_ids: HashMap<winit::window::WindowId, HostedWindowId>,
    next_gpu_retry: Option<Instant>,
    window_hidden: bool,
    render_suspended: bool,
}

impl<Program: HostedProgram> HostedRunner<Program> {
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

impl<Program: HostedProgram> ApplicationHandler<Program::Message> for HostedRunner<Program> {
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
        event: WindowEvent,
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
        let now = Instant::now();
        if ready.graphics.take_device_lost()
            || ready.next_gpu_retry.is_some_and(|deadline| now >= deadline)
        {
            ready.recover_device(event_loop);
        }
        if ready
            .program
            .next_wakeup()
            .is_some_and(|deadline| now >= deadline)
        {
            let context = ready.program_context();
            let update = ready.program.wake(now, &context);
            ready.apply_program_update(event_loop, update);
        }
        let next_wakeup = [ready.next_gpu_retry, ready.program.next_wakeup()]
            .into_iter()
            .flatten()
            .min();
        event_loop.set_control_flow(next_wakeup.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

fn initialize<Program: HostedProgram>(
    event_loop: &ActiveEventLoop,
    proxy: EventLoopProxy<Program::Message>,
    settings: HostedWindowSettings,
) -> Result<HostedReady<Program>, String> {
    let window = Arc::new(
        event_loop
            .create_window(window_attributes(&settings))
            .map_err(|error| format!("failed to create hosted window: {error}"))?,
    );
    if settings.role == HostedWindowRole::Main {
        let _ = prepare_custom_title_bar(window.as_ref());
    }
    let graphics = executor::block_on(HostedGpuContext::new(Arc::clone(&window)))
        .map_err(|error| error.to_string())?;
    let iced_window_id = iced::window::Id::unique();
    let context = program_context(&graphics, &proxy, iced_window_id, false);
    let (program, startup) = Program::initialize(&context).map_err(|error| error.to_string())?;
    let material = material_for(window.as_ref(), program.theme_mode());
    let ui = hosted_ui_renderer(
        &graphics,
        graphics.window(),
        graphics.format(),
        graphics.physical_size(),
    );
    let mut window_ids = HashMap::new();
    window_ids.insert(window.id(), HostedWindowId::PRIMARY);
    let mut ready = HostedReady {
        graphics,
        ui,
        program,
        proxy,
        iced_window_id,
        material,
        settings,
        auxiliary: HashMap::new(),
        window_ids,
        next_gpu_retry: None,
        window_hidden: false,
        render_suspended: false,
    };
    let event = ready.primary_event(true);
    let context = ready.program_context();
    let update = ready.program.window_event(event, &context);
    ready.apply_program_update(event_loop, update);
    for message in startup {
        ready.process_message(event_loop, message);
    }
    ready.graphics.window().request_redraw();
    Ok(ready)
}

impl<Program: HostedProgram> HostedReady<Program> {
    fn program_context(&self) -> HostedProgramContext<Program::Message> {
        program_context(
            &self.graphics,
            &self.proxy,
            self.iced_window_id,
            self.window_hidden,
        )
    }

    fn primary_event(&self, ready: bool) -> HostedWindowEvent {
        let geometry = window_geometry(self.graphics.window());
        if ready {
            HostedWindowEvent::Ready {
                id: HostedWindowId::PRIMARY,
                window_id: self.iced_window_id,
                geometry,
            }
        } else {
            HostedWindowEvent::Resized {
                id: HostedWindowId::PRIMARY,
                window_id: self.iced_window_id,
                geometry,
            }
        }
    }

    fn handle_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: HostedWindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::RedrawRequested => {
                self.update_interface(event_loop, id);
                self.redraw(event_loop, id);
            }
            WindowEvent::CloseRequested => self.notify_close_requested(event_loop, id),
            WindowEvent::Moved(_) => {
                self.notify_moved(event_loop, id);
                self.push_window_event(id, event);
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.resize(id);
                self.notify_resized(event_loop, id);
                self.push_window_event(id, event);
            }
            WindowEvent::Occluded(hidden) => {
                self.set_hidden(id, hidden);
                let window_id = self.iced_window_id(id);
                let context = self.program_context();
                let update = self.program.window_event(
                    HostedWindowEvent::VisibilityChanged {
                        id,
                        window_id,
                        hidden,
                    },
                    &context,
                );
                self.apply_program_update(event_loop, update);
                self.push_window_event(id, WindowEvent::Occluded(hidden));
            }
            event => self.push_window_event(id, event),
        }
    }

    fn push_window_event(&mut self, id: HostedWindowId, event: WindowEvent) {
        if id == HostedWindowId::PRIMARY {
            if self
                .ui
                .push_window_event(event, self.graphics.window().as_ref())
            {
                self.graphics.window().request_redraw();
            }
        } else if let Some(host) = self.auxiliary.get_mut(&id)
            && host
                .ui
                .push_window_event(event, host.surface.window().as_ref())
        {
            host.surface.window().request_redraw();
        }
    }

    fn resize(&mut self, id: HostedWindowId) {
        if id == HostedWindowId::PRIMARY {
            self.graphics.resize();
            self.ui.resize(
                self.graphics.physical_size(),
                self.graphics.window().scale_factor() as f32,
            );
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            self.graphics.resize_surface(&mut host.surface);
            host.ui.resize(
                host.surface.physical_size(),
                host.surface.window().scale_factor() as f32,
            );
        }
    }

    fn notify_resized(&mut self, event_loop: &ActiveEventLoop, id: HostedWindowId) {
        let Some((window_id, geometry)) = self.window_snapshot(id) else {
            return;
        };
        let context = self.program_context();
        let update = self.program.window_event(
            HostedWindowEvent::Resized {
                id,
                window_id,
                geometry,
            },
            &context,
        );
        self.apply_program_update(event_loop, update);
        self.request_redraw(id);
    }

    fn notify_moved(&mut self, event_loop: &ActiveEventLoop, id: HostedWindowId) {
        let Some((window_id, geometry)) = self.window_snapshot(id) else {
            return;
        };
        let context = self.program_context();
        let update = self.program.window_event(
            HostedWindowEvent::Moved {
                id,
                window_id,
                geometry,
            },
            &context,
        );
        self.apply_program_update(event_loop, update);
    }

    fn notify_close_requested(&mut self, event_loop: &ActiveEventLoop, id: HostedWindowId) {
        let window_id = self.iced_window_id(id);
        let context = self.program_context();
        let update = self.program.window_event(
            HostedWindowEvent::CloseRequested { id, window_id },
            &context,
        );
        self.apply_program_update(event_loop, update);
    }

    fn update_interface(&mut self, event_loop: &ActiveEventLoop, id: HostedWindowId) {
        if id == HostedWindowId::PRIMARY {
            if !self.ui.has_pending_events() {
                return;
            }
            let messages = self.ui.update(
                self.program.view_window(id, self.material.is_native()),
                self.graphics.window().as_ref(),
            );
            for message in messages {
                self.process_message(event_loop, message);
            }
            return;
        }
        let Some(host) = self.auxiliary.get_mut(&id) else {
            return;
        };
        if !host.ui.has_pending_events() {
            return;
        }
        let messages = host.ui.update(
            self.program.view_window(id, host.material.is_native()),
            host.surface.window().as_ref(),
        );
        for message in messages {
            self.process_message(event_loop, message);
        }
    }

    fn process_message(&mut self, event_loop: &ActiveEventLoop, message: Program::Message) {
        let previous_theme = self.program.theme_mode();
        let context = self.program_context();
        let update = self.program.update(message, &context);
        if self.program.theme_mode() != previous_theme {
            self.refresh_materials();
        }
        self.apply_program_update(event_loop, update);
    }

    fn apply_program_update(&mut self, event_loop: &ActiveEventLoop, update: HostedProgramUpdate) {
        if update.exit {
            event_loop.exit();
            return;
        }
        for command in update.window_commands {
            self.apply_window_command(event_loop, command);
        }
        if let Some(action) = update.window_action {
            self.apply_window_action(event_loop, action);
        }
        match update.redraw {
            HostedRedraw::None => {}
            HostedRedraw::Primary => self.request_redraw(HostedWindowId::PRIMARY),
            HostedRedraw::Window(id) => self.request_redraw(id),
            HostedRedraw::All => self.request_redraw_all(),
        }
    }

    fn apply_window_command(&mut self, event_loop: &ActiveEventLoop, command: HostedWindowCommand) {
        match command {
            HostedWindowCommand::Open { id, settings } => {
                if id == HostedWindowId::PRIMARY {
                    self.graphics.window().focus_window();
                    return;
                }
                if let Some(host) = self.auxiliary.get(&id) {
                    host.surface.window().focus_window();
                    return;
                }
                match self.open_window(event_loop, id, settings) {
                    Ok(event) => {
                        let context = self.program_context();
                        let update = self.program.window_event(event, &context);
                        self.apply_program_update(event_loop, update);
                    }
                    Err(message) => {
                        let context = self.program_context();
                        let update = self.program.runtime_event(
                            HostedRuntimeEvent::WindowOpenFailed { id, message },
                            &context,
                        );
                        self.apply_program_update(event_loop, update);
                    }
                }
            }
            HostedWindowCommand::Close(id) => self.close_window(id),
            HostedWindowCommand::Focus(id) => self.focus_window(id),
        }
    }

    fn open_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: HostedWindowId,
        settings: HostedWindowSettings,
    ) -> Result<HostedWindowEvent, String> {
        let window = Arc::new(
            event_loop
                .create_window(window_attributes(&settings))
                .map_err(|error| error.to_string())?,
        );
        if settings.role == HostedWindowRole::Main {
            let _ = prepare_custom_title_bar(window.as_ref());
        }
        let surface = self
            .graphics
            .create_surface(window.clone())
            .map_err(|error| error.to_string())?;
        let ui = hosted_ui_renderer(
            &self.graphics,
            surface.window(),
            surface.format(),
            surface.physical_size(),
        );
        let iced_window_id = iced::window::Id::unique();
        let material = material_for(window.as_ref(), self.program.theme_mode());
        let geometry = window_geometry(&window);
        self.window_ids.insert(window.id(), id);
        self.auxiliary.insert(
            id,
            HostedAuxiliary {
                surface,
                ui,
                iced_window_id,
                material,
            },
        );
        window.request_redraw();
        Ok(HostedWindowEvent::Ready {
            id,
            window_id: iced_window_id,
            geometry,
        })
    }

    fn close_window(&mut self, id: HostedWindowId) {
        if id == HostedWindowId::PRIMARY {
            return;
        }
        if let Some(host) = self.auxiliary.remove(&id) {
            self.window_ids.remove(&host.surface.window().id());
        }
    }

    fn focus_window(&self, id: HostedWindowId) {
        if id == HostedWindowId::PRIMARY {
            self.graphics.window().focus_window();
        } else if let Some(host) = self.auxiliary.get(&id) {
            host.surface.window().focus_window();
        }
    }

    fn apply_window_action(&mut self, event_loop: &ActiveEventLoop, action: HostedWindowAction) {
        let Some(window) = self.window(action.id).cloned() else {
            return;
        };
        match action.action {
            WindowChromeAction::Drag => {
                #[cfg(target_os = "macos")]
                let _ = nana_window::drag_custom_title_bar(window.as_ref());
                #[cfg(not(target_os = "macos"))]
                let _ = window.drag_window();
            }
            WindowChromeAction::Minimize => window.set_minimized(true),
            WindowChromeAction::ToggleMaximize => {
                window.set_maximized(!window.is_maximized());
                self.notify_resized(event_loop, action.id);
            }
            WindowChromeAction::Close => self.notify_close_requested(event_loop, action.id),
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop, id: HostedWindowId) {
        if self.render_suspended {
            return;
        }
        if id == HostedWindowId::PRIMARY {
            self.redraw_primary(event_loop);
        } else {
            self.redraw_auxiliary(event_loop, id);
        }
    }

    fn redraw_primary(&mut self, event_loop: &ActiveEventLoop) {
        let frame = match self.graphics.acquire_frame() {
            Ok(HostedSurfaceFrame::Ready(frame)) => frame,
            Ok(HostedSurfaceFrame::Retry) => {
                self.graphics.window().request_redraw();
                return;
            }
            Ok(HostedSurfaceFrame::Skipped) => return,
            Err(error) => {
                self.suspend_rendering(event_loop, error);
                return;
            }
        };
        let context = self.program_context();
        self.program
            .prepare_window_frame(HostedWindowId::PRIMARY, &context);
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let theme_mode = self.program.theme_mode();
        let colors = theme_mode.colors();
        let ui_frame = self.ui.render(
            self.program
                .view_window(HostedWindowId::PRIMARY, self.material.is_native()),
            &theme_mode.iced_theme(),
            renderer::Style {
                text_color: colors.text,
            },
            HostedUiTarget {
                window: self.graphics.window().as_ref(),
                clear_color: Some(window_background(
                    colors.background,
                    self.material.is_native(),
                )),
                format: frame.texture.format(),
                view: &target,
            },
        );
        self.graphics.present(frame);
        for message in ui_frame.messages {
            let _ = self.proxy.send_event(message);
        }
        let context = self.program_context();
        let update =
            self.program
                .window_frame_presented(HostedWindowId::PRIMARY, self.material, &context);
        self.apply_program_update(event_loop, update);
    }

    fn redraw_auxiliary(&mut self, event_loop: &ActiveEventLoop, id: HostedWindowId) {
        let context = self.program_context();
        self.program.prepare_window_frame(id, &context);
        let frame = {
            let Some(host) = self.auxiliary.get_mut(&id) else {
                return;
            };
            match self.graphics.acquire_surface_frame(&mut host.surface) {
                Ok(HostedSurfaceFrame::Ready(frame)) => frame,
                Ok(HostedSurfaceFrame::Retry) => {
                    host.surface.window().request_redraw();
                    return;
                }
                Ok(HostedSurfaceFrame::Skipped) => return,
                Err(error) => {
                    self.suspend_rendering(event_loop, error);
                    return;
                }
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let theme_mode = self.program.theme_mode();
        let colors = theme_mode.colors();
        let material = self.auxiliary[&id].material;
        let ui_frame = {
            let host = self.auxiliary.get_mut(&id).expect("hosted auxiliary");
            host.ui.render(
                self.program.view_window(id, material.is_native()),
                &theme_mode.iced_theme(),
                renderer::Style {
                    text_color: colors.text,
                },
                HostedUiTarget {
                    window: host.surface.window().as_ref(),
                    clear_color: Some(window_background(colors.background, material.is_native())),
                    format: frame.texture.format(),
                    view: &target,
                },
            )
        };
        self.graphics.present(frame);
        for message in ui_frame.messages {
            let _ = self.proxy.send_event(message);
        }
        let context = self.program_context();
        let update = self.program.window_frame_presented(id, material, &context);
        self.apply_program_update(event_loop, update);
    }

    fn suspend_rendering(&mut self, event_loop: &ActiveEventLoop, error: HostedGpuError) {
        self.render_suspended = true;
        self.next_gpu_retry = Some(Instant::now() + self.settings.gpu_retry_interval);
        let context = self.program_context();
        let update = self
            .program
            .runtime_event(HostedRuntimeEvent::RenderingSuspended(error), &context);
        self.apply_program_update(event_loop, update);
    }

    fn refresh_materials(&mut self) {
        clear_system_material(self.graphics.window().as_ref());
        self.material = material_for(self.graphics.window().as_ref(), self.program.theme_mode());
        for host in self.auxiliary.values_mut() {
            clear_system_material(host.surface.window().as_ref());
            host.material = material_for(host.surface.window().as_ref(), self.program.theme_mode());
            host.surface.window().request_redraw();
        }
    }

    fn recover_device(&mut self, event_loop: &ActiveEventLoop) {
        let primary_window = Arc::clone(self.graphics.window());
        match executor::block_on(HostedGpuContext::new(primary_window)) {
            Ok(graphics) => {
                let ui = hosted_ui_renderer(
                    &graphics,
                    graphics.window(),
                    graphics.format(),
                    graphics.physical_size(),
                );
                let mut rebuilt = HashMap::new();
                let previous = std::mem::take(&mut self.auxiliary);
                let mut failures = Vec::new();
                for (id, host) in previous {
                    let window = Arc::clone(host.surface.window());
                    match graphics.create_surface(window) {
                        Ok(surface) => {
                            let ui = hosted_ui_renderer(
                                &graphics,
                                surface.window(),
                                surface.format(),
                                surface.physical_size(),
                            );
                            rebuilt.insert(
                                id,
                                HostedAuxiliary {
                                    surface,
                                    ui,
                                    iced_window_id: host.iced_window_id,
                                    material: host.material,
                                },
                            );
                        }
                        Err(error) => {
                            failures.push((id, host.surface.window().id(), error.to_string()))
                        }
                    }
                }
                self.graphics = graphics;
                self.ui = ui;
                self.auxiliary = rebuilt;
                self.refresh_materials();
                self.next_gpu_retry = None;
                self.render_suspended = false;
                let context = self.program_context();
                self.program.rebuild_gpu(&context);
                let update = self
                    .program
                    .runtime_event(HostedRuntimeEvent::DeviceRecovered, &context);
                self.apply_program_update(event_loop, update);
                for (id, window_id, message) in failures {
                    self.window_ids.remove(&window_id);
                    let context = self.program_context();
                    let update = self.program.runtime_event(
                        HostedRuntimeEvent::WindowOpenFailed { id, message },
                        &context,
                    );
                    self.apply_program_update(event_loop, update);
                }
                self.request_redraw_all();
            }
            Err(error) => {
                self.next_gpu_retry = Some(Instant::now() + self.settings.gpu_retry_interval);
                let context = self.program_context();
                let update = self
                    .program
                    .runtime_event(HostedRuntimeEvent::DeviceRecoveryFailed(error), &context);
                self.apply_program_update(event_loop, update);
            }
        }
    }

    fn set_hidden(&mut self, id: HostedWindowId, hidden: bool) {
        if id == HostedWindowId::PRIMARY {
            self.window_hidden = hidden;
        }
    }

    fn window(&self, id: HostedWindowId) -> Option<&Arc<winit::window::Window>> {
        if id == HostedWindowId::PRIMARY {
            Some(self.graphics.window())
        } else {
            self.auxiliary.get(&id).map(|host| host.surface.window())
        }
    }

    fn iced_window_id(&self, id: HostedWindowId) -> iced::window::Id {
        if id == HostedWindowId::PRIMARY {
            self.iced_window_id
        } else {
            self.auxiliary
                .get(&id)
                .map_or(self.iced_window_id, |host| host.iced_window_id)
        }
    }

    fn window_snapshot(
        &self,
        id: HostedWindowId,
    ) -> Option<(iced::window::Id, HostedWindowGeometry)> {
        self.window(id)
            .map(|window| (self.iced_window_id(id), window_geometry(window)))
    }

    fn request_redraw(&self, id: HostedWindowId) {
        if let Some(window) = self.window(id) {
            window.request_redraw();
        }
    }

    fn request_redraw_all(&self) {
        self.graphics.window().request_redraw();
        for host in self.auxiliary.values() {
            host.surface.window().request_redraw();
        }
    }
}

impl<Program: HostedProgram> Drop for HostedReady<Program> {
    fn drop(&mut self) {
        clear_system_material(self.graphics.window().as_ref());
    }
}

fn program_context<Message: Send + 'static>(
    graphics: &HostedGpuContext,
    proxy: &EventLoopProxy<Message>,
    window_id: iced::window::Id,
    window_hidden: bool,
) -> HostedProgramContext<Message> {
    HostedProgramContext {
        proxy: proxy.clone(),
        gpu: graphics.resources(),
        window_id,
        geometry: window_geometry(graphics.window()),
        window_hidden,
        drawable: graphics.is_drawable(),
        surface_format: graphics.format(),
    }
}

fn hosted_ui_renderer(
    graphics: &HostedGpuContext,
    window: &Arc<winit::window::Window>,
    format: wgpu::TextureFormat,
    physical_size: Size<u32>,
) -> HostedUiRenderer {
    let redraw_window = Arc::downgrade(window);
    HostedUiRenderer::new(
        graphics.adapter(),
        graphics.resources().device(),
        graphics.resources().queue(),
        format,
        physical_size,
        window.scale_factor() as f32,
        move || {
            if let Some(window) = redraw_window.upgrade() {
                window.request_redraw();
            }
        },
    )
}

fn window_geometry(window: &winit::window::Window) -> HostedWindowGeometry {
    let scale_factor = normalized_scale_factor(window.scale_factor() as f32);
    let physical_size = window.inner_size();
    let physical_position = window.outer_position().ok();
    HostedWindowGeometry {
        physical_position: physical_position.map(|position| (position.x, position.y)),
        physical_size: Size::new(physical_size.width, physical_size.height),
        logical_position: physical_position.map(|position| {
            let logical = position.to_logical::<f32>(f64::from(scale_factor));
            Point::new(logical.x, logical.y)
        }),
        logical_size: Size::new(
            physical_size.width as f32 / scale_factor,
            physical_size.height as f32 / scale_factor,
        ),
        scale_factor,
        maximized: window.is_maximized(),
    }
}

fn normalized_scale_factor(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

#[cfg(target_os = "macos")]
fn material_for(window: &winit::window::Window, _theme: ThemeMode) -> MaterialOutcome {
    clear_system_material(window);
    MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
}

#[cfg(not(target_os = "macos"))]
fn material_for(window: &winit::window::Window, theme: ThemeMode) -> MaterialOutcome {
    let (appearance, fallback) = match theme {
        ThemeMode::Dark => (Appearance::Dark, FallbackColor::rgba(24, 24, 24, 220)),
        ThemeMode::Light => (Appearance::Light, FallbackColor::rgba(255, 255, 255, 232)),
    };
    apply_system_material(window, appearance, fallback)
}

fn window_background(mut color: iced::Color, native_material: bool) -> iced::Color {
    if native_material {
        color.a = 0.78;
    }
    color
}

fn window_attributes(settings: &HostedWindowSettings) -> winit::window::WindowAttributes {
    let mut attributes = winit::window::WindowAttributes::default()
        .with_title(settings.title.clone())
        .with_transparent(settings.transparent)
        .with_inner_size(winit::dpi::LogicalSize::new(
            settings.initial_size.width,
            settings.initial_size.height,
        ))
        .with_min_inner_size(winit::dpi::LogicalSize::new(
            settings.minimum_size.width,
            settings.minimum_size.height,
        ))
        .with_maximized(settings.maximized);
    if let Some((x, y, width, height)) = settings.initial_physical_geometry {
        attributes = attributes
            .with_position(winit::dpi::PhysicalPosition::new(x, y))
            .with_inner_size(winit::dpi::PhysicalSize::new(width, height));
    } else if let Some((x, y)) = settings.initial_position {
        attributes = attributes.with_position(winit::dpi::LogicalPosition::new(x, y));
    }

    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;

        if settings.role == HostedWindowRole::Tool {
            attributes.with_decorations(true)
        } else {
            attributes
                .with_decorations(true)
                .with_title_hidden(true)
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        attributes.with_decorations(matches!(settings.role, HostedWindowRole::Tool))
    }
}

#[derive(Debug)]
pub enum HostedRunError {
    EventLoop(winit::error::EventLoopError),
    Startup(String),
}

impl fmt::Display for HostedRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(error) => write!(formatter, "hosted event loop failed: {error}"),
            Self::Startup(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for HostedRunError {}

#[cfg(test)]
mod tests {
    use super::{HostedProgramUpdate, HostedRedraw, HostedWindowId, HostedWindowSettings};

    #[test]
    fn redraw_targets_are_explicit() {
        assert_eq!(HostedProgramUpdate::default().redraw, HostedRedraw::None);
        assert_eq!(HostedProgramUpdate::redraw().redraw, HostedRedraw::Primary);
        assert_eq!(
            HostedProgramUpdate::redraw_window(HostedWindowId(7)).redraw,
            HostedRedraw::Window(HostedWindowId(7))
        );
        assert_eq!(HostedProgramUpdate::redraw_all().redraw, HostedRedraw::All);
    }

    #[test]
    fn physical_geometry_overrides_logical_restoration() {
        let settings = HostedWindowSettings::new("test")
            .initial_position(10.0, 20.0)
            .initial_size(640.0, 480.0)
            .physical_geometry(20, 40, 1280, 960);
        assert_eq!(
            settings.initial_physical_geometry,
            Some((20, 40, 1280, 960))
        );
    }
}
