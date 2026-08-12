//! High-level native runtime for applications that host NanaUI in one WGPU context.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
use std::sync::mpsc::{self, Receiver, Sender};
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
use nana_window::{
    MaterialEffect, MaterialOutcome, clear_system_material, prepare_custom_title_bar,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};

#[cfg(feature = "browser")]
use crate::layout_probe::LayoutBounds;
use crate::theme::ThemeModeExt;
use crate::{
    AppearanceSettings, BackdropTarget, HostedGpuContext, HostedGpuError, HostedGpuResources,
    HostedGpuSurface, HostedSurfaceFrame, HostedUiRenderer, HostedUiTarget, ThemeMode,
    WindowChromeAction, WindowMaterialMode,
};

const HOSTED_REDRAW_SETTLE_PASSES: usize = 3;

/// Stable application-owned identity for one hosted window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostedWindowId(pub u64);

impl HostedWindowId {
    pub const PRIMARY: Self = Self(0);
}

/// Application-owned identity for one asynchronous hosted-window capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostedWindowCaptureId(pub u64);

/// Stable application-owned identity for one child browser surface.
#[cfg(feature = "browser")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostedBrowserId(pub u64);

/// Logical bounds for a child browser, relative to its hosted parent window.
#[cfg(feature = "browser")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostedBrowserBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[cfg(feature = "browser")]
impl HostedBrowserBounds {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn validate(self) -> Result<Self, &'static str> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || !self.width.is_finite()
            || !self.height.is_finite()
        {
            return Err("browser bounds must be finite");
        }
        if self.width <= 0.0 || self.height <= 0.0 {
            return Err("browser bounds must have a positive size");
        }
        Ok(self)
    }
}

#[cfg(feature = "browser")]
impl From<LayoutBounds> for HostedBrowserBounds {
    fn from(bounds: LayoutBounds) -> Self {
        Self::new(
            f64::from(bounds.x),
            f64::from(bounds.y),
            f64::from(bounds.width),
            f64::from(bounds.height),
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HostedWindowRole {
    #[default]
    Main,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedTitleBarMode {
    Custom,
    Native,
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
    pub transparent_background: bool,
    pub always_on_top: bool,
    pub resizable: bool,
    pub role: HostedWindowRole,
    pub title_bar_mode: HostedTitleBarMode,
    pub gpu_retry_interval: Duration,
    pub required_gpu_features: wgpu::Features,
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
            transparent_background: false,
            always_on_top: false,
            resizable: true,
            role: HostedWindowRole::Main,
            title_bar_mode: HostedTitleBarMode::Custom,
            gpu_retry_interval: Duration::from_secs(2),
            required_gpu_features: wgpu::Features::empty(),
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

    /// Uses a fully transparent surface clear and disables native backdrop materials.
    pub fn transparent_background(mut self, transparent_background: bool) -> Self {
        self.transparent_background = transparent_background;
        if transparent_background {
            self.transparent = true;
        }
        self
    }

    pub fn always_on_top(mut self, always_on_top: bool) -> Self {
        self.always_on_top = always_on_top;
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    pub fn required_gpu_features(mut self, features: wgpu::Features) -> Self {
        self.required_gpu_features = features;
        self
    }

    pub fn role(mut self, role: HostedWindowRole) -> Self {
        self.role = role;
        self
    }

    pub fn tool_window(mut self) -> Self {
        self.role = HostedWindowRole::Tool;
        self.title_bar_mode = HostedTitleBarMode::Native;
        self
    }

    pub fn title_bar_mode(mut self, mode: HostedTitleBarMode) -> Self {
        self.title_bar_mode = mode;
        self
    }

    pub fn custom_title_bar(mut self) -> Self {
        self.title_bar_mode = HostedTitleBarMode::Custom;
        self
    }

    pub fn native_title_bar(mut self) -> Self {
        self.title_bar_mode = HostedTitleBarMode::Native;
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
#[derive(Debug, Clone)]
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
    /// Native window keyboard focus (winit `Focused`). Distinct from Page Visibility.
    FocusChanged {
        id: HostedWindowId,
        window_id: iced::window::Id,
        focused: bool,
    },
    CloseRequested {
        id: HostedWindowId,
        window_id: iced::window::Id,
    },
    FileHovered {
        id: HostedWindowId,
        window_id: iced::window::Id,
        path: PathBuf,
        position: Option<Point>,
    },
    FileDropped {
        id: HostedWindowId,
        window_id: iced::window::Id,
        path: PathBuf,
        position: Option<Point>,
    },
    FileHoverCancelled {
        id: HostedWindowId,
        window_id: iced::window::Id,
    },
}

impl HostedWindowEvent {
    pub const fn id(&self) -> HostedWindowId {
        match self {
            Self::Ready { id, .. }
            | Self::Resized { id, .. }
            | Self::Moved { id, .. }
            | Self::VisibilityChanged { id, .. }
            | Self::FocusChanged { id, .. }
            | Self::CloseRequested { id, .. }
            | Self::FileHovered { id, .. }
            | Self::FileDropped { id, .. }
            | Self::FileHoverCancelled { id, .. } => *id,
        }
    }
}

/// Typed host failures and recoveries. User-facing copy remains app-owned.
#[derive(Debug, Clone)]
pub enum HostedRuntimeEvent {
    RenderingSuspended(HostedGpuError),
    DeviceRecovered,
    DeviceRecoveryFailed(HostedGpuError),
    WindowOpenFailed {
        id: HostedWindowId,
        message: String,
    },
    WindowCaptured {
        id: HostedWindowId,
        capture_id: HostedWindowCaptureId,
        path: PathBuf,
    },
    WindowCaptureFailed {
        id: HostedWindowId,
        capture_id: HostedWindowCaptureId,
        message: String,
    },
    #[cfg(feature = "browser")]
    Browser(HostedBrowserEvent),
}

#[cfg(feature = "browser")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedBrowserLoadState {
    Started,
    Finished,
}

#[cfg(feature = "browser")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedBrowserCommandKind {
    Attach,
    Navigate,
    SetBounds,
    SetVisible,
    Focus,
    Detach,
}

/// Browser lifecycle notifications translated out of the platform webview.
#[cfg(feature = "browser")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostedBrowserEvent {
    PageLoad {
        id: HostedBrowserId,
        state: HostedBrowserLoadState,
        url: String,
    },
    DocumentTitleChanged {
        id: HostedBrowserId,
        title: String,
    },
    CommandFailed {
        id: HostedBrowserId,
        command: HostedBrowserCommandKind,
        message: String,
    },
}

#[cfg(feature = "browser")]
impl HostedBrowserEvent {
    pub const fn id(&self) -> HostedBrowserId {
        match self {
            Self::PageLoad { id, .. }
            | Self::DocumentTitleChanged { id, .. }
            | Self::CommandFailed { id, .. } => *id,
        }
    }
}

/// Commands for an application-owned browser surface inside a hosted window.
#[cfg(feature = "browser")]
#[derive(Debug, Clone)]
pub enum HostedBrowserCommand {
    Attach {
        id: HostedBrowserId,
        window_id: HostedWindowId,
        url: String,
        bounds: HostedBrowserBounds,
    },
    Navigate {
        id: HostedBrowserId,
        url: String,
    },
    SetBounds {
        id: HostedBrowserId,
        bounds: HostedBrowserBounds,
    },
    SetVisible {
        id: HostedBrowserId,
        visible: bool,
    },
    Focus(HostedBrowserId),
    Detach(HostedBrowserId),
}

#[cfg(feature = "browser")]
impl HostedBrowserCommand {
    const fn identity(&self) -> (HostedBrowserId, HostedBrowserCommandKind) {
        match self {
            Self::Attach { id, .. } => (*id, HostedBrowserCommandKind::Attach),
            Self::Navigate { id, .. } => (*id, HostedBrowserCommandKind::Navigate),
            Self::SetBounds { id, .. } => (*id, HostedBrowserCommandKind::SetBounds),
            Self::SetVisible { id, .. } => (*id, HostedBrowserCommandKind::SetVisible),
            Self::Focus(id) => (*id, HostedBrowserCommandKind::Focus),
            Self::Detach(id) => (*id, HostedBrowserCommandKind::Detach),
        }
    }
}

#[derive(Debug, Clone)]
pub enum HostedWindowCommand {
    Open {
        id: HostedWindowId,
        settings: HostedWindowSettings,
    },
    Close(HostedWindowId),
    Move {
        id: HostedWindowId,
        position: Point,
    },
    Focus(HostedWindowId),
    CapturePng {
        id: HostedWindowId,
        capture_id: HostedWindowCaptureId,
        path: PathBuf,
    },
    #[cfg(feature = "browser")]
    Browser(HostedBrowserCommand),
}

/// Commands that operate on retained NanaUI widget state after the next rebuild.
#[derive(Debug, Clone)]
pub enum HostedUiCommand {
    Focus {
        window_id: HostedWindowId,
        target: String,
    },
    ScrollBy {
        window_id: HostedWindowId,
        target: String,
        x: f32,
        y: f32,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HostedRedraw {
    #[default]
    None,
    Primary,
    Window(HostedWindowId),
    All,
    DynamicPrimary,
    DynamicWindow(HostedWindowId),
    DynamicAll,
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
    pub ui_commands: Vec<HostedUiCommand>,
    pub exit: bool,
}

impl HostedProgramUpdate {
    pub const fn redraw() -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::Primary,
            window_commands: Vec::new(),
            ui_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn redraw_window(id: HostedWindowId) -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::Window(id),
            window_commands: Vec::new(),
            ui_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn redraw_all() -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::All,
            window_commands: Vec::new(),
            ui_commands: Vec::new(),
            exit: false,
        }
    }

    /// Requests a frame for dynamic host content without rebuilding the UI.
    pub const fn redraw_dynamic() -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::DynamicPrimary,
            window_commands: Vec::new(),
            ui_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn redraw_dynamic_window(id: HostedWindowId) -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::DynamicWindow(id),
            window_commands: Vec::new(),
            ui_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn redraw_dynamic_all() -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::DynamicAll,
            window_commands: Vec::new(),
            ui_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn exit() -> Self {
        Self {
            window_action: None,
            redraw: HostedRedraw::None,
            window_commands: Vec::new(),
            ui_commands: Vec::new(),
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
            ui_commands: Vec::new(),
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

    pub fn with_ui_commands(mut self, commands: impl IntoIterator<Item = HostedUiCommand>) -> Self {
        self.ui_commands.extend(commands);
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

    /// Build the primary window tree.
    ///
    /// Returns `Element<'static, …>` because [`crate::HostedUiRenderer`] retains
    /// an iced `UserInterface<'static>` across frames. Prefer owning snapshots
    /// (e.g. [`crate::DesktopShell`] with `WorkspaceController::clone`) over
    /// borrowing live program fields into the element tree.
    fn view(&self, native_material: bool) -> Element<'static, Self::Message>;

    fn view_window(
        &self,
        id: HostedWindowId,
        native_material: bool,
    ) -> Element<'static, Self::Message> {
        let _ = id;
        self.view(native_material)
    }

    fn theme_mode(&self) -> ThemeMode;

    /// Preferred window material from Appearance settings.
    ///
    /// Defaults to [`WindowMaterialMode::Translucent`] so hosted windows that
    /// keep the platform-transparent default still receive Mica/Acrylic (or the
    /// documented solid fallback). Hosts that wire [`AppearanceSettings`] should
    /// return `appearance.window_material()` instead.
    ///
    /// Hosts apply this through `nana-window`; widgets never receive handles.
    fn window_material_mode(&self) -> WindowMaterialMode {
        WindowMaterialMode::Translucent
    }

    /// Foreground cover opacity used when a native material is active.
    fn backdrop_opacity(&self) -> f32 {
        AppearanceSettings::DEFAULT_BACKDROP_OPACITY
    }

    /// Which shell region reveals the translucent material.
    ///
    /// Defaults to sidebar. Hosts that wire [`AppearanceSettings`] should return
    /// `appearance.backdrop_target()`.
    fn backdrop_target(&self) -> BackdropTarget {
        BackdropTarget::Sidebar
    }

    /// When the sidebar is translucent, whether the title bar shares that alpha.
    ///
    /// Defaults to `true` (Lilia / Appearance default). Hosts should return
    /// `appearance.titlebar_follows_sidebar()`.
    fn titlebar_follows_sidebar(&self) -> bool {
        true
    }

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

struct HostedAuxiliary<Message> {
    surface: HostedGpuSurface,
    ui: HostedUiRenderer<Message>,
    iced_window_id: iced::window::Id,
    material: MaterialOutcome,
    settings: HostedWindowSettings,
}

#[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
struct HostedBrowserHost {
    window_id: HostedWindowId,
    webview: wry::WebView,
}

impl<Message> Drop for HostedAuxiliary<Message> {
    fn drop(&mut self) {
        clear_system_material(self.surface.window().as_ref());
    }
}

struct HostedReady<Program: HostedProgram> {
    #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
    browsers: HashMap<HostedBrowserId, HostedBrowserHost>,
    #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
    browser_events_tx: Sender<HostedBrowserEvent>,
    #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
    browser_events_rx: Receiver<HostedBrowserEvent>,
    graphics: HostedGpuContext,
    ui: HostedUiRenderer<Program::Message>,
    program: Program,
    proxy: EventLoopProxy<Program::Message>,
    iced_window_id: iced::window::Id,
    material: MaterialOutcome,
    settings: HostedWindowSettings,
    auxiliary: HashMap<HostedWindowId, HostedAuxiliary<Program::Message>>,
    window_ids: HashMap<winit::window::WindowId, HostedWindowId>,
    cursor_positions: HashMap<HostedWindowId, Point>,
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
        #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
        ready.drain_browser_events(event_loop);
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
    if settings.title_bar_mode == HostedTitleBarMode::Custom {
        let _ = prepare_custom_title_bar(window.as_ref());
    }
    let graphics = executor::block_on(HostedGpuContext::new(
        Arc::clone(&window),
        settings.required_gpu_features,
    ))
    .map_err(|error| error.to_string())?;
    let iced_window_id = iced::window::Id::unique();
    let context = program_context(&graphics, &proxy, iced_window_id, false);
    let (program, startup) = Program::initialize(&context).map_err(|error| error.to_string())?;
    let material = material_for(
        window.as_ref(),
        program.theme_mode(),
        settings.transparent_background,
    );
    let ui = hosted_ui_renderer(
        &graphics,
        graphics.window(),
        graphics.format(),
        graphics.physical_size(),
    );
    let mut window_ids = HashMap::new();
    window_ids.insert(window.id(), HostedWindowId::PRIMARY);
    #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
    let (browser_events_tx, browser_events_rx) = mpsc::channel();
    let mut ready = HostedReady {
        #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
        browsers: HashMap::new(),
        #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
        browser_events_tx,
        #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
        browser_events_rx,
        graphics,
        ui,
        program,
        proxy,
        iced_window_id,
        material,
        settings,
        auxiliary: HashMap::new(),
        window_ids,
        cursor_positions: HashMap::new(),
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
            WindowEvent::Focused(focused) => {
                self.notify_focus_changed(event_loop, id, focused);
                self.push_window_event(id, WindowEvent::Focused(focused));
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(scale_factor) = self.window_scale_factor(id) {
                    let position = position.to_logical::<f32>(f64::from(scale_factor));
                    self.cursor_positions
                        .insert(id, Point::new(position.x, position.y));
                }
                self.push_window_event(id, event);
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_positions.remove(&id);
                self.push_window_event(id, event);
            }
            WindowEvent::HoveredFile(path) => {
                self.notify_file_hovered(event_loop, id, path.clone());
                self.push_window_event(id, WindowEvent::HoveredFile(path));
            }
            WindowEvent::DroppedFile(path) => {
                self.notify_file_dropped(event_loop, id, path.clone());
                self.push_window_event(id, WindowEvent::DroppedFile(path));
            }
            WindowEvent::HoveredFileCancelled => {
                self.notify_file_hover_cancelled(event_loop, id);
                self.push_window_event(id, WindowEvent::HoveredFileCancelled);
            }
            event => self.push_window_event(id, event),
        }
    }

    fn window_scale_factor(&self, id: HostedWindowId) -> Option<f32> {
        if id == HostedWindowId::PRIMARY {
            Some(self.graphics.window().scale_factor() as f32)
        } else {
            self.auxiliary
                .get(&id)
                .map(|host| host.surface.window().scale_factor() as f32)
        }
    }

    fn notify_file_hovered(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: HostedWindowId,
        path: PathBuf,
    ) {
        let window_id = self.iced_window_id(id);
        let position = self.cursor_positions.get(&id).copied();
        let context = self.program_context();
        let update = self.program.window_event(
            HostedWindowEvent::FileHovered {
                id,
                window_id,
                path,
                position,
            },
            &context,
        );
        self.apply_program_update(event_loop, update);
    }

    fn notify_file_dropped(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: HostedWindowId,
        path: PathBuf,
    ) {
        let window_id = self.iced_window_id(id);
        let position = self.cursor_positions.get(&id).copied();
        let context = self.program_context();
        let update = self.program.window_event(
            HostedWindowEvent::FileDropped {
                id,
                window_id,
                path,
                position,
            },
            &context,
        );
        self.apply_program_update(event_loop, update);
    }

    fn notify_file_hover_cancelled(&mut self, event_loop: &ActiveEventLoop, id: HostedWindowId) {
        let window_id = self.iced_window_id(id);
        let context = self.program_context();
        let update = self.program.window_event(
            HostedWindowEvent::FileHoverCancelled { id, window_id },
            &context,
        );
        self.apply_program_update(event_loop, update);
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

    fn notify_focus_changed(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: HostedWindowId,
        focused: bool,
    ) {
        let window_id = self.iced_window_id(id);
        let context = self.program_context();
        let update = self.program.window_event(
            HostedWindowEvent::FocusChanged {
                id,
                window_id,
                focused,
            },
            &context,
        );
        self.apply_program_update(event_loop, update);
    }

    fn ensure_ui(&mut self, id: HostedWindowId) {
        if id == HostedWindowId::PRIMARY {
            if self.ui.is_ui_dirty() {
                let content = self.program.view_window(id, self.material.is_native());
                self.ui.rebuild(content);
            }
            return;
        }

        let needs_rebuild = self
            .auxiliary
            .get(&id)
            .is_some_and(|host| host.ui.is_ui_dirty());
        if !needs_rebuild {
            return;
        }
        let Some(material) = self.auxiliary.get(&id).map(|host| host.material) else {
            return;
        };
        let content = self.program.view_window(id, material.is_native());
        if let Some(host) = self.auxiliary.get_mut(&id) {
            host.ui.rebuild(content);
        }
    }
    fn process_message(&mut self, event_loop: &ActiveEventLoop, message: Program::Message) {
        self.process_message_inner(event_loop, message, None);
    }

    fn process_input_message(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: HostedWindowId,
        message: Program::Message,
    ) {
        self.process_message_inner(event_loop, message, Some(id));
    }

    fn process_message_inner(
        &mut self,
        event_loop: &ActiveEventLoop,
        message: Program::Message,
        redraw_satisfied_by: Option<HostedWindowId>,
    ) {
        let previous_theme = self.program.theme_mode();
        let previous_material = self.program.window_material_mode();
        let previous_opacity = self.program.backdrop_opacity();
        let previous_target = self.program.backdrop_target();
        let previous_titlebar_follows = self.program.titlebar_follows_sidebar();
        let context = self.program_context();
        let update = self.program.update(message, &context);
        if self.program.theme_mode() != previous_theme
            || self.program.window_material_mode() != previous_material
        {
            self.refresh_materials();
        } else if (self.program.backdrop_opacity() - previous_opacity).abs() > f32::EPSILON
            || self.program.backdrop_target() != previous_target
            || self.program.titlebar_follows_sidebar() != previous_titlebar_follows
        {
            self.ui.mark_ui_dirty();
            for host in self.auxiliary.values_mut() {
                host.ui.mark_ui_dirty();
                host.surface.window().request_redraw();
            }
            self.graphics.window().request_redraw();
        }
        self.apply_program_update_inner(event_loop, update, redraw_satisfied_by);
    }

    fn apply_program_update(&mut self, event_loop: &ActiveEventLoop, update: HostedProgramUpdate) {
        self.apply_program_update_inner(event_loop, update, None);
    }

    fn apply_program_update_inner(
        &mut self,
        event_loop: &ActiveEventLoop,
        update: HostedProgramUpdate,
        redraw_satisfied_by: Option<HostedWindowId>,
    ) {
        if update.exit {
            event_loop.exit();
            return;
        }
        for command in update.window_commands {
            self.apply_window_command(event_loop, command);
        }
        for command in update.ui_commands {
            self.apply_ui_command(command);
        }
        if let Some(action) = update.window_action {
            self.apply_window_action(event_loop, action);
        }
        self.request_program_redraw(update.redraw, redraw_satisfied_by);
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
            HostedWindowCommand::Move { id, position } => self.move_window(id, position),
            HostedWindowCommand::Focus(id) => self.focus_window(id),
            HostedWindowCommand::CapturePng {
                id,
                capture_id,
                path,
            } => {
                let event = match self.capture_window_png(id, &path) {
                    Ok(()) => HostedRuntimeEvent::WindowCaptured {
                        id,
                        capture_id,
                        path,
                    },
                    Err(message) => HostedRuntimeEvent::WindowCaptureFailed {
                        id,
                        capture_id,
                        message,
                    },
                };
                let context = self.program_context();
                let update = self.program.runtime_event(event, &context);
                self.apply_program_update(event_loop, update);
            }
            #[cfg(feature = "browser")]
            HostedWindowCommand::Browser(command) => {
                self.apply_browser_command(event_loop, command);
            }
        }
    }

    #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
    fn apply_browser_command(
        &mut self,
        event_loop: &ActiveEventLoop,
        command: HostedBrowserCommand,
    ) {
        let (id, kind) = command.identity();
        let result = match command {
            HostedBrowserCommand::Attach {
                id,
                window_id,
                url,
                bounds,
            } => self.attach_browser(id, window_id, url, bounds),
            HostedBrowserCommand::Navigate { id, url } => self
                .browsers
                .get(&id)
                .ok_or_else(|| format!("browser {} is not attached", id.0))
                .and_then(|host| {
                    host.webview
                        .load_url(&url)
                        .map_err(|error| error.to_string())
                }),
            HostedBrowserCommand::SetBounds { id, bounds } => {
                let bounds = bounds.validate().map_err(String::from);
                bounds.and_then(|bounds| {
                    self.browsers
                        .get(&id)
                        .ok_or_else(|| format!("browser {} is not attached", id.0))?
                        .webview
                        .set_bounds(browser_rect(bounds))
                        .map_err(|error| error.to_string())
                })
            }
            HostedBrowserCommand::SetVisible { id, visible } => self
                .browsers
                .get(&id)
                .ok_or_else(|| format!("browser {} is not attached", id.0))
                .and_then(|host| {
                    host.webview
                        .set_visible(visible)
                        .map_err(|error| error.to_string())
                }),
            HostedBrowserCommand::Focus(id) => self
                .browsers
                .get(&id)
                .ok_or_else(|| format!("browser {} is not attached", id.0))
                .and_then(|host| host.webview.focus().map_err(|error| error.to_string())),
            HostedBrowserCommand::Detach(id) => {
                self.browsers.remove(&id);
                Ok(())
            }
        };
        if let Err(message) = result {
            self.emit_browser_event(
                event_loop,
                HostedBrowserEvent::CommandFailed {
                    id,
                    command: kind,
                    message,
                },
            );
        }
    }

    #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
    fn attach_browser(
        &mut self,
        id: HostedBrowserId,
        window_id: HostedWindowId,
        url: String,
        bounds: HostedBrowserBounds,
    ) -> Result<(), String> {
        if self.browsers.contains_key(&id) {
            return Err(format!("browser {} is already attached", id.0));
        }
        let bounds = bounds.validate().map_err(String::from)?;
        let window = self
            .window(window_id)
            .cloned()
            .ok_or_else(|| format!("hosted window {} does not exist", window_id.0))?;
        let load_events = self.browser_events_tx.clone();
        let load_event_window = window.clone();
        let title_events = self.browser_events_tx.clone();
        let title_event_window = window.clone();
        let webview = wry::WebViewBuilder::new()
            .with_url(url)
            .with_bounds(browser_rect(bounds))
            .with_on_page_load_handler(move |state, url| {
                let state = match state {
                    wry::PageLoadEvent::Started => HostedBrowserLoadState::Started,
                    wry::PageLoadEvent::Finished => HostedBrowserLoadState::Finished,
                };
                if load_events
                    .send(HostedBrowserEvent::PageLoad { id, state, url })
                    .is_ok()
                {
                    load_event_window.request_redraw();
                }
            })
            .with_document_title_changed_handler(move |title| {
                if title_events
                    .send(HostedBrowserEvent::DocumentTitleChanged { id, title })
                    .is_ok()
                {
                    title_event_window.request_redraw();
                }
            })
            .build_as_child(window.as_ref())
            .map_err(|error| error.to_string())?;
        self.browsers
            .insert(id, HostedBrowserHost { window_id, webview });
        Ok(())
    }

    #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
    fn drain_browser_events(&mut self, event_loop: &ActiveEventLoop) {
        let events = self.browser_events_rx.try_iter().collect::<Vec<_>>();
        for event in events {
            if self.browsers.contains_key(&event.id()) {
                self.emit_browser_event(event_loop, event);
            }
        }
    }

    #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
    fn emit_browser_event(&mut self, event_loop: &ActiveEventLoop, event: HostedBrowserEvent) {
        let context = self.program_context();
        let update = self
            .program
            .runtime_event(HostedRuntimeEvent::Browser(event), &context);
        self.apply_program_update(event_loop, update);
    }

    #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
    fn detach_browsers_for_window(&mut self, window_id: HostedWindowId) {
        self.browsers
            .retain(|_, browser| browser.window_id != window_id);
    }

    #[cfg(all(
        feature = "browser",
        not(any(target_os = "windows", target_os = "macos"))
    ))]
    fn apply_browser_command(
        &mut self,
        event_loop: &ActiveEventLoop,
        command: HostedBrowserCommand,
    ) {
        let (id, kind) = command.identity();
        let context = self.program_context();
        let update = self.program.runtime_event(
            HostedRuntimeEvent::Browser(HostedBrowserEvent::CommandFailed {
                id,
                command: kind,
                message: "hosted browsers are supported on Windows and macOS".to_owned(),
            }),
            &context,
        );
        self.apply_program_update(event_loop, update);
    }

    fn apply_ui_command(&mut self, command: HostedUiCommand) {
        match command {
            HostedUiCommand::Focus { window_id, target } => {
                if window_id == HostedWindowId::PRIMARY {
                    self.ui.queue_focus(target);
                } else if let Some(host) = self.auxiliary.get_mut(&window_id) {
                    host.ui.queue_focus(target);
                }
            }
            HostedUiCommand::ScrollBy {
                window_id,
                target,
                x,
                y,
            } => {
                if window_id == HostedWindowId::PRIMARY {
                    self.ui.queue_scroll_by(target, x, y);
                } else if let Some(host) = self.auxiliary.get_mut(&window_id) {
                    host.ui.queue_scroll_by(target, x, y);
                }
            }
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
        if settings.title_bar_mode == HostedTitleBarMode::Custom {
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
        let material = material_for(
            window.as_ref(),
            self.program.theme_mode(),
            settings.transparent_background,
        );
        let geometry = window_geometry(&window);
        self.window_ids.insert(window.id(), id);
        self.auxiliary.insert(
            id,
            HostedAuxiliary {
                surface,
                ui,
                iced_window_id,
                material,
                settings,
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
        #[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
        self.detach_browsers_for_window(id);
        if let Some(host) = self.auxiliary.remove(&id) {
            self.cursor_positions.remove(&id);
            self.window_ids.remove(&host.surface.window().id());
        }
    }

    fn focus_window(&self, id: HostedWindowId) {
        if id == HostedWindowId::PRIMARY {
            self.graphics.window().set_visible(true);
            self.graphics.window().focus_window();
        } else if let Some(host) = self.auxiliary.get(&id) {
            host.surface.window().set_visible(true);
            host.surface.window().focus_window();
        }
    }

    fn move_window(&self, id: HostedWindowId, position: Point) {
        let Some(window) = self.window(id) else {
            return;
        };
        window.set_outer_position(winit::dpi::Position::Logical(
            winit::dpi::LogicalPosition::new(f64::from(position.x), f64::from(position.y)),
        ));
    }

    #[cfg(target_os = "windows")]
    fn capture_window_png(&self, id: HostedWindowId, path: &std::path::Path) -> Result<(), String> {
        use image::{ImageBuffer, Rgba};
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use std::fs;
        use std::ptr::null_mut;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap,
            CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP,
            HDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
        };
        use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

        struct WindowDc {
            hwnd: HWND,
            hdc: HDC,
        }
        impl Drop for WindowDc {
            fn drop(&mut self) {
                unsafe {
                    let _ = ReleaseDC(Some(self.hwnd), self.hdc);
                }
            }
        }

        struct MemoryDc(HDC);
        impl Drop for MemoryDc {
            fn drop(&mut self) {
                unsafe {
                    let _ = DeleteDC(self.0);
                }
            }
        }

        struct Bitmap(HBITMAP);
        impl Drop for Bitmap {
            fn drop(&mut self) {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(self.0.0));
                }
            }
        }

        struct SelectedObject {
            hdc: HDC,
            previous: HGDIOBJ,
        }
        impl Drop for SelectedObject {
            fn drop(&mut self) {
                unsafe {
                    let _ = SelectObject(self.hdc, self.previous);
                }
            }
        }

        let window = self
            .window(id)
            .ok_or_else(|| format!("hosted window {} does not exist", id.0))?;
        let handle = window
            .window_handle()
            .map_err(|error| format!("failed to read hosted window handle: {error}"))?;
        let hwnd = match handle.as_raw() {
            RawWindowHandle::Win32(raw) => HWND(raw.hwnd.get() as *mut _),
            _ => return Err("hosted window does not expose a Win32 handle".to_owned()),
        };

        unsafe {
            let mut rect = Default::default();
            GetClientRect(hwnd, &mut rect)
                .map_err(|error| format!("failed to read hosted window bounds: {error}"))?;
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width <= 0 || height <= 0 {
                return Err("hosted window has no drawable client area".to_owned());
            }

            let hdc = GetDC(Some(hwnd));
            if hdc.0 == null_mut() {
                return Err("failed to acquire hosted window device context".to_owned());
            }
            let window_dc = WindowDc { hwnd, hdc };
            let memory_dc = CreateCompatibleDC(Some(window_dc.hdc));
            if memory_dc.0 == null_mut() {
                return Err("failed to create capture device context".to_owned());
            }
            let memory_dc = MemoryDc(memory_dc);
            let bitmap = CreateCompatibleBitmap(window_dc.hdc, width, height);
            if bitmap.0 == null_mut() {
                return Err("failed to create capture bitmap".to_owned());
            }
            let bitmap = Bitmap(bitmap);
            let previous = SelectObject(memory_dc.0, HGDIOBJ(bitmap.0.0));
            if previous.0 == null_mut() {
                return Err("failed to select capture bitmap".to_owned());
            }
            let _selected = SelectedObject {
                hdc: memory_dc.0,
                previous,
            };
            BitBlt(
                memory_dc.0,
                0,
                0,
                width,
                height,
                Some(window_dc.hdc),
                0,
                0,
                SRCCOPY,
            )
            .map_err(|error| format!("failed to copy hosted window pixels: {error}"))?;

            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bgra = vec![0_u8; (width as usize) * (height as usize) * 4];
            let rows = GetDIBits(
                memory_dc.0,
                bitmap.0,
                0,
                height as u32,
                Some(bgra.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            );
            if rows == 0 {
                return Err("failed to read captured hosted window pixels".to_owned());
            }
            for pixel in bgra.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            let image = ImageBuffer::<Rgba<u8>, _>::from_raw(width as u32, height as u32, bgra)
                .ok_or_else(|| "failed to assemble hosted window capture".to_owned())?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to create capture directory: {error}"))?;
            }
            image
                .save(path)
                .map_err(|error| format!("failed to save hosted window capture: {error}"))?;
        }
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    fn capture_window_png(
        &self,
        id: HostedWindowId,
        _path: &std::path::Path,
    ) -> Result<(), String> {
        if self.window(id).is_none() {
            return Err(format!("hosted window {} does not exist", id.0));
        }
        Err("hosted window PNG capture is currently supported on Windows".to_owned())
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
        let mut passes = 0;
        let mut first_pass = true;
        let prepared = 'settle: loop {
            self.ensure_ui(HostedWindowId::PRIMARY);
            let context = self.program_context();
            self.program
                .prepare_window_frame(HostedWindowId::PRIMARY, &context);
            let mut prepared = if first_pass {
                self.ui
                    .prepare_frame(self.graphics.window().as_ref(), Instant::now())
            } else {
                self.ui
                    .prepare_redraw(self.graphics.window().as_ref(), Instant::now())
            };
            first_pass = false;

            loop {
                passes += 1;
                let message_count = prepared.message_count();
                let has_layout_changed = prepared.has_layout_changed();
                if message_count > 0 {
                    if passes >= HOSTED_REDRAW_SETTLE_PASSES {
                        break 'settle prepared;
                    }
                    let messages = self
                        .ui
                        .cache_prepared(prepared, self.graphics.window().as_ref());
                    for message in messages {
                        self.process_input_message(event_loop, HostedWindowId::PRIMARY, message);
                    }
                    continue 'settle;
                }
                if !has_layout_changed {
                    break 'settle prepared;
                }
                if passes >= HOSTED_REDRAW_SETTLE_PASSES {
                    break 'settle prepared;
                }
                self.ui.update_prepared_redraw(
                    &mut prepared,
                    self.graphics.window().as_ref(),
                    Instant::now(),
                );
            }
        };
        let frame = match self.graphics.acquire_frame() {
            Ok(HostedSurfaceFrame::Ready(frame)) => frame,
            Ok(HostedSurfaceFrame::Retry) => {
                let messages = self
                    .ui
                    .cache_prepared(prepared, self.graphics.window().as_ref());
                for message in messages {
                    let _ = self.proxy.send_event(message);
                }
                self.graphics.window().request_redraw();
                return;
            }
            Ok(HostedSurfaceFrame::Skipped) => {
                let messages = self
                    .ui
                    .cache_prepared(prepared, self.graphics.window().as_ref());
                for message in messages {
                    let _ = self.proxy.send_event(message);
                }
                return;
            }
            Err(error) => {
                let messages = self
                    .ui
                    .cache_prepared(prepared, self.graphics.window().as_ref());
                for message in messages {
                    let _ = self.proxy.send_event(message);
                }
                self.suspend_rendering(event_loop, error);
                return;
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let theme_mode = self.program.theme_mode();
        let colors = theme_mode.colors();
        let ui_frame = self.ui.present_prepared(
            prepared,
            &theme_mode.iced_theme(),
            renderer::Style {
                text_color: colors.text,
            },
            HostedUiTarget {
                window: self.graphics.window().as_ref(),
                clear_color: Some(window_background(colors.background, self.material)),
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
        let mut passes = 0;
        let mut first_pass = true;
        let (prepared, material) = 'settle: loop {
            let Some((window, material)) = self
                .auxiliary
                .get(&id)
                .map(|host| (Arc::clone(host.surface.window()), host.material))
            else {
                return;
            };
            self.ensure_ui(id);
            let context = self.program_context();
            self.program.prepare_window_frame(id, &context);
            let mut prepared = {
                let host = self.auxiliary.get_mut(&id).expect("hosted auxiliary");
                if first_pass {
                    host.ui.prepare_frame(window.as_ref(), Instant::now())
                } else {
                    host.ui.prepare_redraw(window.as_ref(), Instant::now())
                }
            };
            first_pass = false;

            loop {
                passes += 1;
                let message_count = prepared.message_count();
                let has_layout_changed = prepared.has_layout_changed();
                if message_count > 0 {
                    if passes >= HOSTED_REDRAW_SETTLE_PASSES {
                        break 'settle (prepared, material);
                    }
                    let messages = {
                        let host = self.auxiliary.get_mut(&id).expect("hosted auxiliary");
                        host.ui.cache_prepared(prepared, window.as_ref())
                    };
                    for message in messages {
                        self.process_input_message(event_loop, id, message);
                    }
                    continue 'settle;
                }
                if !has_layout_changed {
                    break 'settle (prepared, material);
                }
                if passes >= HOSTED_REDRAW_SETTLE_PASSES {
                    break 'settle (prepared, material);
                }
                {
                    let host = self.auxiliary.get_mut(&id).expect("hosted auxiliary");
                    host.ui
                        .update_prepared_redraw(&mut prepared, window.as_ref(), Instant::now());
                }
            }
        };
        let frame = {
            let Some(host) = self.auxiliary.get_mut(&id) else {
                return;
            };
            match self.graphics.acquire_surface_frame(&mut host.surface) {
                Ok(HostedSurfaceFrame::Ready(frame)) => frame,
                Ok(HostedSurfaceFrame::Retry) => {
                    let messages = host
                        .ui
                        .cache_prepared(prepared, host.surface.window().as_ref());
                    for message in messages {
                        let _ = self.proxy.send_event(message);
                    }
                    host.surface.window().request_redraw();
                    return;
                }
                Ok(HostedSurfaceFrame::Skipped) => {
                    let messages = host
                        .ui
                        .cache_prepared(prepared, host.surface.window().as_ref());
                    for message in messages {
                        let _ = self.proxy.send_event(message);
                    }
                    return;
                }
                Err(error) => {
                    let messages = host
                        .ui
                        .cache_prepared(prepared, host.surface.window().as_ref());
                    for message in messages {
                        let _ = self.proxy.send_event(message);
                    }
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
        let ui_frame = {
            let host = self.auxiliary.get_mut(&id).expect("hosted auxiliary");
            host.ui.present_prepared(
                prepared,
                &theme_mode.iced_theme(),
                renderer::Style {
                    text_color: colors.text,
                },
                HostedUiTarget {
                    window: host.surface.window().as_ref(),
                    clear_color: Some(window_background(colors.background, material)),
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
        self.material = material_for(
            self.graphics.window().as_ref(),
            self.program.theme_mode(),
            self.settings.transparent_background,
        );
        self.ui.mark_ui_dirty();
        for host in self.auxiliary.values_mut() {
            clear_system_material(host.surface.window().as_ref());
            host.material = material_for(
                host.surface.window().as_ref(),
                self.program.theme_mode(),
                host.settings.transparent_background,
            );
            host.ui.mark_ui_dirty();
            host.surface.window().request_redraw();
        }
    }

    fn recover_device(&mut self, event_loop: &ActiveEventLoop) {
        let primary_window = Arc::clone(self.graphics.window());
        match executor::block_on(HostedGpuContext::new(
            primary_window,
            self.settings.required_gpu_features,
        )) {
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
                                    settings: host.settings.clone(),
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
                    #[cfg(all(
                        feature = "browser",
                        any(target_os = "windows", target_os = "macos")
                    ))]
                    self.detach_browsers_for_window(id);
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
            self.ui.mark_ui_dirty();
            if !hidden {
                self.ui.mark_dynamic_dirty();
                self.graphics.window().request_redraw();
            }
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            host.ui.mark_ui_dirty();
            if !hidden {
                host.ui.mark_dynamic_dirty();
                host.surface.window().request_redraw();
            }
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

    fn request_program_redraw(
        &mut self,
        redraw: HostedRedraw,
        redraw_satisfied_by: Option<HostedWindowId>,
    ) {
        self.mark_program_dirty(redraw);
        if should_request_redraw(redraw, HostedWindowId::PRIMARY, redraw_satisfied_by) {
            self.request_redraw(HostedWindowId::PRIMARY);
        }
        for id in self.auxiliary.keys().copied() {
            if should_request_redraw(redraw, id, redraw_satisfied_by) {
                self.request_redraw(id);
            }
        }
    }

    fn mark_program_dirty(&mut self, redraw: HostedRedraw) {
        match redraw {
            HostedRedraw::None => {}
            HostedRedraw::Primary => self.ui.mark_ui_dirty(),
            HostedRedraw::DynamicPrimary => self.ui.mark_dynamic_dirty(),
            HostedRedraw::Window(id) => self.mark_window_ui_dirty(id),
            HostedRedraw::DynamicWindow(id) => self.mark_window_dynamic_dirty(id),
            HostedRedraw::All => {
                self.ui.mark_ui_dirty();
                for host in self.auxiliary.values_mut() {
                    host.ui.mark_ui_dirty();
                }
            }
            HostedRedraw::DynamicAll => {
                self.ui.mark_dynamic_dirty();
                for host in self.auxiliary.values_mut() {
                    host.ui.mark_dynamic_dirty();
                }
            }
        }
    }

    fn mark_window_ui_dirty(&mut self, id: HostedWindowId) {
        if id == HostedWindowId::PRIMARY {
            self.ui.mark_ui_dirty();
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            host.ui.mark_ui_dirty();
        }
    }

    fn mark_window_dynamic_dirty(&mut self, id: HostedWindowId) {
        if id == HostedWindowId::PRIMARY {
            self.ui.mark_dynamic_dirty();
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            host.ui.mark_dynamic_dirty();
        }
    }

    fn request_redraw_all(&self) {
        self.graphics.window().request_redraw();
        for host in self.auxiliary.values() {
            host.surface.window().request_redraw();
        }
    }
}

fn should_request_redraw(
    redraw: HostedRedraw,
    target: HostedWindowId,
    redraw_satisfied_by: Option<HostedWindowId>,
) -> bool {
    if redraw_satisfied_by == Some(target) {
        return false;
    }
    match redraw {
        HostedRedraw::None => false,
        HostedRedraw::Primary | HostedRedraw::DynamicPrimary => target == HostedWindowId::PRIMARY,
        HostedRedraw::Window(id) | HostedRedraw::DynamicWindow(id) => target == id,
        HostedRedraw::All | HostedRedraw::DynamicAll => true,
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

fn hosted_ui_renderer<Message>(
    graphics: &HostedGpuContext,
    window: &Arc<winit::window::Window>,
    format: wgpu::TextureFormat,
    physical_size: Size<u32>,
) -> HostedUiRenderer<Message> {
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

#[cfg(all(feature = "browser", any(target_os = "windows", target_os = "macos")))]
fn browser_rect(bounds: HostedBrowserBounds) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(bounds.x, bounds.y).into(),
        size: wry::dpi::LogicalSize::new(bounds.width, bounds.height).into(),
    }
}

fn normalized_scale_factor(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

fn material_for(
    window: &winit::window::Window,
    theme: ThemeMode,
    transparent_background: bool,
) -> MaterialOutcome {
    if transparent_background {
        clear_system_material(window);
        return MaterialOutcome::transparent();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = theme;
        clear_system_material(window);
        MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let (appearance, fallback) = match theme {
            ThemeMode::Dark => (Appearance::Dark, FallbackColor::rgba(24, 24, 24, 220)),
            ThemeMode::Light => (Appearance::Light, FallbackColor::rgba(255, 255, 255, 232)),
        };
        apply_system_material(window, appearance, fallback)
    }
}

fn window_background(mut color: iced::Color, material: MaterialOutcome) -> iced::Color {
    if material.effect == MaterialEffect::Transparent {
        return iced::Color::TRANSPARENT;
    }
    if material.is_native() {
        color.a = 0.78;
    }
    color
}

fn window_attributes(settings: &HostedWindowSettings) -> winit::window::WindowAttributes {
    let mut attributes = winit::window::WindowAttributes::default()
        .with_title(settings.title.clone())
        .with_transparent(settings.transparent || settings.transparent_background)
        .with_resizable(settings.resizable)
        .with_window_level(if settings.always_on_top {
            winit::window::WindowLevel::AlwaysOnTop
        } else {
            winit::window::WindowLevel::Normal
        })
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

        if settings.title_bar_mode == HostedTitleBarMode::Native {
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
        attributes.with_decorations(settings.title_bar_mode == HostedTitleBarMode::Native)
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
    #[cfg(feature = "browser")]
    use super::{
        HostedBrowserBounds, HostedBrowserCommand, HostedBrowserCommandKind, HostedBrowserId,
    };
    use super::{
        HostedProgramUpdate, HostedRedraw, HostedTitleBarMode, HostedWindowEvent, HostedWindowId,
        HostedWindowRole, HostedWindowSettings, should_request_redraw, window_attributes,
        window_background,
    };
    use iced::Point;
    use iced_winit::winit;
    use nana_window::{MaterialEffect, MaterialFallback, MaterialOutcome};
    use std::path::PathBuf;

    #[cfg(feature = "browser")]
    #[test]
    fn browser_commands_keep_application_identity_and_reject_invalid_geometry() {
        let browser_id = HostedBrowserId(12);
        let measured = crate::LayoutBounds::new(640.0, 44.0, 480.0, 720.0);
        let command = HostedBrowserCommand::Attach {
            id: browser_id,
            window_id: HostedWindowId::PRIMARY,
            url: "https://example.com".to_owned(),
            bounds: measured.into(),
        };

        assert_eq!(
            command.identity(),
            (browser_id, HostedBrowserCommandKind::Attach)
        );
        let converted = HostedBrowserBounds::from(measured);
        assert_eq!(
            converted,
            HostedBrowserBounds::new(640.0, 44.0, 480.0, 720.0)
        );
        assert!(
            HostedBrowserBounds::new(-12.0, 44.0, 480.0, 720.0)
                .validate()
                .is_ok()
        );
        assert!(
            HostedBrowserBounds::new(0.0, 0.0, 0.0, 720.0)
                .validate()
                .is_err()
        );
        assert!(
            HostedBrowserBounds::new(0.0, f64::NAN, 480.0, 720.0)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn file_drop_event_preserves_window_path_and_cursor_position() {
        let event = HostedWindowEvent::FileDropped {
            id: HostedWindowId(7),
            window_id: iced::window::Id::unique(),
            path: PathBuf::from("C:/workspace/screenshot.png"),
            position: Some(Point::new(24.0, 48.0)),
        };

        assert_eq!(event.id(), HostedWindowId(7));
        match event {
            HostedWindowEvent::FileDropped { path, position, .. } => {
                assert_eq!(path, PathBuf::from("C:/workspace/screenshot.png"));
                assert_eq!(position, Some(Point::new(24.0, 48.0)));
            }
            _ => unreachable!("constructed a file-drop event"),
        }
    }

    #[test]
    fn redraw_targets_are_explicit() {
        assert_eq!(HostedProgramUpdate::default().redraw, HostedRedraw::None);
        assert_eq!(HostedProgramUpdate::redraw().redraw, HostedRedraw::Primary);
        assert_eq!(
            HostedProgramUpdate::redraw_window(HostedWindowId(7)).redraw,
            HostedRedraw::Window(HostedWindowId(7))
        );
        assert_eq!(HostedProgramUpdate::redraw_all().redraw, HostedRedraw::All);
        assert_eq!(
            HostedProgramUpdate::redraw_dynamic().redraw,
            HostedRedraw::DynamicPrimary
        );
        assert_eq!(
            HostedProgramUpdate::redraw_dynamic_window(HostedWindowId(7)).redraw,
            HostedRedraw::DynamicWindow(HostedWindowId(7))
        );
        assert_eq!(
            HostedProgramUpdate::redraw_dynamic_all().redraw,
            HostedRedraw::DynamicAll
        );
    }

    #[test]
    fn input_frame_satisfies_only_its_own_redraw() {
        let primary = HostedWindowId::PRIMARY;
        let tool = HostedWindowId(7);

        assert!(!should_request_redraw(
            HostedRedraw::Primary,
            primary,
            Some(primary)
        ));
        assert!(should_request_redraw(HostedRedraw::Primary, primary, None));
        assert!(!should_request_redraw(
            HostedRedraw::Window(tool),
            tool,
            Some(tool)
        ));
        assert!(should_request_redraw(
            HostedRedraw::Window(tool),
            tool,
            Some(primary)
        ));
        assert!(should_request_redraw(
            HostedRedraw::DynamicWindow(tool),
            tool,
            Some(primary)
        ));
    }

    #[test]
    fn redraw_all_preserves_every_window_except_the_current_input_frame() {
        let primary = HostedWindowId::PRIMARY;
        let tool = HostedWindowId(7);

        assert!(!should_request_redraw(
            HostedRedraw::All,
            primary,
            Some(primary)
        ));
        assert!(should_request_redraw(
            HostedRedraw::All,
            tool,
            Some(primary)
        ));
        assert!(should_request_redraw(
            HostedRedraw::All,
            primary,
            Some(tool)
        ));
        assert!(!should_request_redraw(HostedRedraw::All, tool, Some(tool)));
        assert!(should_request_redraw(
            HostedRedraw::DynamicAll,
            tool,
            Some(primary)
        ));
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

    #[test]
    fn title_bar_mode_controls_host_window_decorations() {
        let main = HostedWindowSettings::new("test");
        assert_eq!(main.role, HostedWindowRole::Main);
        assert_eq!(main.title_bar_mode, HostedTitleBarMode::Custom);

        let tool = HostedWindowSettings::new("tool").tool_window();
        assert_eq!(tool.role, HostedWindowRole::Tool);
        assert_eq!(tool.title_bar_mode, HostedTitleBarMode::Native);
        assert!(window_attributes(&tool).decorations);

        let custom_tool = HostedWindowSettings::new("tool")
            .tool_window()
            .custom_title_bar();
        assert_eq!(custom_tool.role, HostedWindowRole::Tool);
        assert_eq!(custom_tool.title_bar_mode, HostedTitleBarMode::Custom);

        #[cfg(target_os = "macos")]
        {
            assert!(window_attributes(&main).decorations);
            assert!(window_attributes(&custom_tool).decorations);
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert!(!window_attributes(&main).decorations);
            assert!(!window_attributes(&custom_tool).decorations);
        }
    }

    #[test]
    fn desktop_pet_window_policy_is_explicit() {
        let settings = HostedWindowSettings::new("pet")
            .transparent_background(true)
            .always_on_top(true)
            .resizable(false);
        let attributes = window_attributes(&settings);

        assert!(settings.transparent);
        assert!(settings.transparent_background);
        assert_eq!(
            attributes.window_level,
            winit::window::WindowLevel::AlwaysOnTop
        );
        assert!(!attributes.resizable);
        assert!(attributes.transparent);
    }

    #[test]
    fn transparent_material_clears_the_surface_without_becoming_native() {
        let color = iced::Color::from_rgba(0.2, 0.3, 0.4, 1.0);

        assert_eq!(
            window_background(color, MaterialOutcome::transparent()),
            iced::Color::TRANSPARENT
        );
        assert_eq!(
            window_background(
                color,
                MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
            ),
            color
        );
        assert_eq!(
            window_background(color, MaterialOutcome::native(MaterialEffect::Acrylic)).a,
            0.78
        );
    }
}
