//! High-level native runtime for applications that host NanaUI in one WGPU context.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::{Element, Size};
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
    HostedGpuContext, HostedGpuError, HostedGpuResources, HostedSurfaceFrame, HostedUiRenderer,
    HostedUiTarget, ThemeMode, WindowChromeAction,
};

/// Native window configuration for [`run_hosted`].
#[derive(Debug, Clone)]
pub struct HostedWindowSettings {
    pub title: String,
    pub initial_size: Size<f64>,
    pub minimum_size: Size<f64>,
    pub transparent: bool,
    pub gpu_retry_interval: Duration,
}

impl HostedWindowSettings {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            initial_size: Size::new(1200.0, 800.0),
            minimum_size: Size::new(760.0, 520.0),
            transparent: !cfg!(target_os = "macos"),
            gpu_retry_interval: Duration::from_secs(2),
        }
    }

    pub fn initial_size(mut self, width: f64, height: f64) -> Self {
        self.initial_size = Size::new(width, height);
        self
    }

    pub fn minimum_size(mut self, width: f64, height: f64) -> Self {
        self.minimum_size = Size::new(width, height);
        self
    }

    pub fn transparent(mut self, transparent: bool) -> Self {
        self.transparent = transparent;
        self
    }
}

/// Stable application-facing state for the current native host.
#[derive(Clone)]
pub struct HostedProgramContext<Message: 'static> {
    proxy: EventLoopProxy<Message>,
    gpu: HostedGpuResources,
    window_id: iced::window::Id,
    physical_size: Size<u32>,
    scale_factor: f32,
    maximized: bool,
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

    pub const fn physical_size(&self) -> Size<u32> {
        self.physical_size
    }

    pub fn logical_size(&self) -> Size {
        Size::new(
            self.physical_size.width as f32 / self.scale_factor,
            self.physical_size.height as f32 / self.scale_factor,
        )
    }

    pub const fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    pub const fn maximized(&self) -> bool {
        self.maximized
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

/// Window lifecycle information translated into Iced logical coordinates.
#[derive(Debug, Clone, Copy)]
pub enum HostedWindowEvent {
    Ready {
        window_id: iced::window::Id,
        logical_size: Size,
        scale_factor: f32,
        maximized: bool,
    },
    Resized {
        window_id: iced::window::Id,
        logical_size: Size,
        scale_factor: f32,
        maximized: bool,
    },
    VisibilityChanged {
        window_id: iced::window::Id,
        hidden: bool,
    },
    CloseRequested {
        window_id: iced::window::Id,
    },
}

/// Typed host failures and recoveries. User-facing copy remains app-owned.
#[derive(Debug, Clone)]
pub enum HostedRuntimeEvent {
    RenderingSuspended(HostedGpuError),
    DeviceRecovered,
    DeviceRecoveryFailed(HostedGpuError),
}

/// Commands returned after a program handles one event.
#[derive(Debug, Clone, Copy, Default)]
pub struct HostedProgramUpdate {
    pub window_action: Option<WindowChromeAction>,
    pub request_redraw: bool,
    pub exit: bool,
}

impl HostedProgramUpdate {
    pub const fn redraw() -> Self {
        Self {
            window_action: None,
            request_redraw: true,
            exit: false,
        }
    }

    pub const fn exit() -> Self {
        Self {
            window_action: None,
            request_redraw: false,
            exit: true,
        }
    }

    pub const fn with_window_action(action: WindowChromeAction) -> Self {
        Self {
            window_action: Some(action),
            request_redraw: true,
            exit: false,
        }
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

    /// Renders application-owned GPU targets before NanaUI consumes them.
    fn prepare_frame(&mut self, _context: &HostedProgramContext<Self::Message>) {}

    /// Rebuilds application GPU resources after the host replaces its device.
    fn rebuild_gpu(&mut self, _context: &HostedProgramContext<Self::Message>) {}

    fn frame_presented(
        &mut self,
        _material: MaterialOutcome,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        HostedProgramUpdate::default()
    }
}

/// Starts a native hosted NanaUI application.
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

struct HostedReady<Program: HostedProgram> {
    graphics: HostedGpuContext,
    ui: HostedUiRenderer,
    program: Program,
    proxy: EventLoopProxy<Program::Message>,
    iced_window_id: iced::window::Id,
    material: MaterialOutcome,
    settings: HostedWindowSettings,
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
        let proxy = proxy.clone();
        let settings = settings.clone();
        match initialize::<Program>(event_loop, proxy, settings) {
            Ok(ready) => *self = Self::Ready(Box::new(ready)),
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, message: Program::Message) {
        let Self::Ready(ready) = self else {
            return;
        };
        ready.process_message(event_loop, message);
        ready.graphics.window().request_redraw();
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
        if window_id != ready.graphics.window().id() {
            return;
        }

        match event {
            WindowEvent::RedrawRequested => {
                ready.update_interface(event_loop);
                ready.redraw(event_loop);
            }
            WindowEvent::CloseRequested => ready.notify_close_requested(event_loop),
            event => {
                let geometry_changed = matches!(
                    event,
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
                );
                if geometry_changed {
                    ready.graphics.resize();
                    ready.resize_interface();
                    ready.notify_resized(event_loop);
                }
                if let WindowEvent::Occluded(occluded) = &event {
                    ready.window_hidden = *occluded;
                    let context = ready.program_context();
                    let update = ready.program.window_event(
                        HostedWindowEvent::VisibilityChanged {
                            window_id: ready.iced_window_id,
                            hidden: *occluded,
                        },
                        &context,
                    );
                    ready.apply_program_update(event_loop, update);
                }
                if ready
                    .ui
                    .push_window_event(event, ready.graphics.window().as_ref())
                {
                    ready.graphics.window().request_redraw();
                }
            }
        }
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
    let _ = prepare_custom_title_bar(window.as_ref());
    let graphics = executor::block_on(HostedGpuContext::new(Arc::clone(&window)))
        .map_err(|error| error.to_string())?;
    let iced_window_id = iced::window::Id::unique();
    let context = program_context(&graphics, &proxy, iced_window_id, false);
    let (program, startup) = Program::initialize(&context).map_err(|error| error.to_string())?;
    let material = material_for(window.as_ref(), program.theme_mode());
    let ui = hosted_ui_renderer(&graphics);
    let mut ready = HostedReady {
        graphics,
        ui,
        program,
        proxy,
        iced_window_id,
        material,
        settings,
        next_gpu_retry: None,
        window_hidden: false,
        render_suspended: false,
    };
    let context = ready.program_context();
    let update = ready
        .program
        .window_event(ready.window_event(true), &context);
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

    fn window_event(&self, ready: bool) -> HostedWindowEvent {
        let context = self.program_context();
        if ready {
            HostedWindowEvent::Ready {
                window_id: self.iced_window_id,
                logical_size: context.logical_size(),
                scale_factor: context.scale_factor(),
                maximized: context.maximized(),
            }
        } else {
            HostedWindowEvent::Resized {
                window_id: self.iced_window_id,
                logical_size: context.logical_size(),
                scale_factor: context.scale_factor(),
                maximized: context.maximized(),
            }
        }
    }

    fn notify_resized(&mut self, event_loop: &ActiveEventLoop) {
        let event = self.window_event(false);
        let context = self.program_context();
        let update = self.program.window_event(event, &context);
        self.apply_program_update(event_loop, update);
        self.graphics.window().request_redraw();
    }

    fn notify_close_requested(&mut self, event_loop: &ActiveEventLoop) {
        let context = self.program_context();
        let update = self.program.window_event(
            HostedWindowEvent::CloseRequested {
                window_id: self.iced_window_id,
            },
            &context,
        );
        self.apply_program_update(event_loop, update);
    }

    fn update_interface(&mut self, event_loop: &ActiveEventLoop) {
        if !self.ui.has_pending_events() {
            return;
        }
        let messages = self.ui.update(
            self.program.view(self.material.is_native()),
            self.graphics.window().as_ref(),
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
            self.refresh_material();
        }
        self.apply_program_update(event_loop, update);
    }

    fn apply_program_update(&mut self, event_loop: &ActiveEventLoop, update: HostedProgramUpdate) {
        if update.exit {
            event_loop.exit();
            return;
        }
        if let Some(action) = update.window_action {
            self.apply_window_action(event_loop, action);
        }
        if update.request_redraw {
            self.graphics.window().request_redraw();
        }
    }

    fn apply_window_action(&mut self, event_loop: &ActiveEventLoop, action: WindowChromeAction) {
        match action {
            WindowChromeAction::Drag => {
                #[cfg(target_os = "macos")]
                let _ = nana_window::drag_custom_title_bar(self.graphics.window().as_ref());
                #[cfg(not(target_os = "macos"))]
                let _ = self.graphics.window().drag_window();
            }
            WindowChromeAction::Minimize => self.graphics.window().set_minimized(true),
            WindowChromeAction::ToggleMaximize => {
                self.graphics
                    .window()
                    .set_maximized(!self.graphics.window().is_maximized());
                self.notify_resized(event_loop);
            }
            WindowChromeAction::Close => self.notify_close_requested(event_loop),
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        if self.render_suspended {
            return;
        }
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
        self.program.prepare_frame(&context);
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let theme_mode = self.program.theme_mode();
        let theme = theme_mode.iced_theme();
        let colors = theme_mode.colors();
        let ui_frame = self.ui.render(
            self.program.view(self.material.is_native()),
            &theme,
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
        let update = self.program.frame_presented(self.material, &context);
        self.apply_program_update(event_loop, update);
    }

    fn resize_interface(&mut self) {
        self.ui.resize(
            self.graphics.physical_size(),
            self.graphics.window().scale_factor() as f32,
        );
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

    fn refresh_material(&mut self) {
        clear_system_material(self.graphics.window().as_ref());
        self.material = material_for(self.graphics.window().as_ref(), self.program.theme_mode());
    }

    fn recover_device(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::clone(self.graphics.window());
        match executor::block_on(HostedGpuContext::new(window)) {
            Ok(graphics) => {
                let ui = hosted_ui_renderer(&graphics);
                let context = program_context(
                    &graphics,
                    &self.proxy,
                    self.iced_window_id,
                    self.window_hidden,
                );
                self.program.rebuild_gpu(&context);
                self.ui = ui;
                self.graphics = graphics;
                self.next_gpu_retry = None;
                self.render_suspended = false;
                let context = self.program_context();
                let update = self
                    .program
                    .runtime_event(HostedRuntimeEvent::DeviceRecovered, &context);
                self.apply_program_update(event_loop, update);
                self.graphics.window().request_redraw();
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
    let scale_factor = graphics.window().scale_factor() as f32;
    HostedProgramContext {
        proxy: proxy.clone(),
        gpu: graphics.resources(),
        window_id,
        physical_size: graphics.physical_size(),
        scale_factor: if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        },
        maximized: graphics.window().is_maximized(),
        window_hidden,
        drawable: graphics.is_drawable(),
        surface_format: graphics.format(),
    }
}

fn hosted_ui_renderer(graphics: &HostedGpuContext) -> HostedUiRenderer {
    let redraw_window = Arc::downgrade(graphics.window());
    HostedUiRenderer::new(
        graphics.adapter(),
        graphics.resources().device(),
        graphics.resources().queue(),
        graphics.format(),
        graphics.physical_size(),
        graphics.window().scale_factor() as f32,
        move || {
            if let Some(window) = redraw_window.upgrade() {
                window.request_redraw();
            }
        },
    )
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
    let attributes = winit::window::WindowAttributes::default()
        .with_title(settings.title.clone())
        .with_transparent(settings.transparent)
        .with_inner_size(winit::dpi::LogicalSize::new(
            settings.initial_size.width,
            settings.initial_size.height,
        ))
        .with_min_inner_size(winit::dpi::LogicalSize::new(
            settings.minimum_size.width,
            settings.minimum_size.height,
        ));

    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;

        attributes
            .with_decorations(true)
            .with_title_hidden(true)
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
    }

    #[cfg(not(target_os = "macos"))]
    {
        attributes.with_decorations(false)
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
