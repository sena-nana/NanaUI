//! Nana-owned winit + wgpu loop for [`crate::RuntimeProgram`].
//!
//! Paint goes through [`crate::SceneWgpuPainter`].

mod accessibility;
mod browser;
mod color_sampler;
mod damage;
mod dialogs;
mod display;
mod embedded;
mod host_services;
mod input;
mod manager;
mod output;
mod pipeline_cache;
mod presence;
mod present;
mod production;
mod runner;
mod schedule;
mod startup;
mod tasks;
pub(crate) use tasks::spawn_detached_task_workers;
mod web_surface;
mod window_factory;
mod windows;

#[cfg(test)]
mod tests;

pub use embedded::EmbeddedRuntime;

use accessibility::PendingAccessibility;
use host_services::{NativeWindowServices, WindowInputSource};
use manager::*;
use runner::SceneRunner;
use tasks::spawn_task_workers;
use window_factory::*;

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::presentation::{ResolvedWindowPresentation, WindowSurfaceTarget};
use crate::winit_input::WinitInputAdapter;
use nana_ui_core::{AppearanceSettings, RESIZE_HANDLE_SIZE, SharedStore, TITLE_BAR_HEIGHT};
use nana_ui_platform::host::WindowCommand;
use nana_ui_platform::{
    DisplayBounds, FileDragInput, FullscreenRequest, InputPayload, MousePassthroughMode,
    PointerInput, PointerPhase, PointerType, WindowEvent, WindowGeometry, WindowIcon, WindowId,
    WindowLevel, WindowModeState, WindowResizeEdge, WindowRole, clamp_position_to_displays,
    clear_registered_application_icon, persist_live_window_geometry, register_application_icon,
    restore_window_geometry, window_resize_edge,
};
use nana_ui_runtime::{
    AccessibilityUpdate, Entity, FrameworkError, LayoutViewport, StableNodeId, Task,
};
#[cfg(target_os = "macos")]
use nana_window::set_application_icon_png;
use nana_window::{
    Appearance, FallbackColor, FrameResizeEdge, LiveSizeMove, MaterialOutcome,
    apply_hosted_system_material, arm_frameless_guard, clear_system_material,
    prepare_client_chrome, resize_custom_frame, set_frameless_styles,
};
use winit::application::ApplicationHandler;
use winit::cursor::CursorIcon;
use winit::event::WindowEvent as WinitWindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::icon::{Icon, RgbaIcon};
#[cfg(target_os = "macos")]
use winit::platform::macos::{WindowAttributesMacOS, WindowExtMacOS};
#[cfg(target_os = "windows")]
use winit::platform::windows::{CornerPreference, WindowAttributesWindows, WindowExtWindows};
use winit::raw_window_handle::HasWindowHandle;

#[cfg(not(target_os = "android"))]
use crate::accessibility::HostedAccessibility;
use crate::gpu_raw::GpuRaw;
use crate::nana_text::NanaTextShaper;
use crate::runtime_host::{
    HostDocumentAccess, HostFailure, ReportHostFailure, RuntimeProgram, RuntimeProgramContext,
    RuntimeProgramUpdate, RuntimeRedraw, WindowDescriptor,
};
use crate::scene_paint::{ScenePaintViewport, SceneWgpuPainter};
use crate::{
    HostTextureRegistry, HostedGpuError, HostedGpuSurface, HostedRunError, RuntimeAnimationClock,
    TitleBarDragTracker, WindowChromeAction, WindowChromeEvent, WindowChromeState,
    apply_title_bar_pointer, title_bar_hits_window_control as pointer_hits_window_control,
    window_commands_for_chrome_action,
};

const GPU_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const MAX_PROGRAM_DISPATCHES: usize = 32;
const TASK_QUEUE_CAPACITY: usize = 256;
const TASK_WORKERS: usize = 4;

/// Run a [`RuntimeProgram`] on the Nana Scene host.
///
/// Startup options set with [`crate::with_startup`] (or
/// [`crate::run_runtime_with_startup`]) on this thread apply to this host.
pub fn run_runtime_scene<Program: RuntimeProgram>(
    settings: WindowDescriptor,
) -> Result<(), HostedRunError> {
    let entry = Instant::now();
    let options = crate::runtime_host::take_pending_startup();
    startup::startup_event(startup::StartupMark::Entry, Duration::ZERO);
    let event_loop = EventLoop::new().map_err(HostedRunError::EventLoop)?;
    event_loop.set_control_flow(ControlFlow::Wait);
    if nana_diagnostics::global().is_some() {
        nana_diagnostics::stall::watch_current_thread();
    }
    let (message_tx, message_rx) = mpsc::channel();
    let startup_failure = Arc::new(Mutex::new(None));
    let runner = SceneRunner::<Program>::Loading(Box::new(startup::LoadingStartup {
        proxy: event_loop.create_proxy(),
        message_tx,
        message_rx,
        settings,
        startup_failure: Arc::clone(&startup_failure),
        options,
        entry,
    }));
    let run = event_loop.run_app(runner);
    nana_diagnostics::event!(nana_diagnostics::framework::host::EVENT_LOOP_EXITED);
    run.map_err(HostedRunError::EventLoop)?;
    match startup_failure.lock().ok().and_then(|guard| guard.clone()) {
        Some(message) => Err(HostedRunError::Startup(message)),
        None => Ok(()),
    }
}

/// What mirroring a window's scene into the platform compositor has cost since
/// the host started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompositionWork {
    pub native_content: crate::NativeContentWork,
    pub commits: usize,
    pub tree_mutations: usize,
    pub native_chrome_writes: usize,
}

struct WindowContext {
    surface_retry: Option<Instant>,
    surface_generation: u64,
    applied_appearance: Option<windows::WindowAppearance>,
    cursor_override: Option<host_services::NativeCursor>,
    cursor_visible_override: Option<bool>,
    passthrough_mode: MousePassthroughMode,
    os_mouse_passthrough: bool,
    material_override: Option<nana_window::MaterialEffect>,
    surface: HostedGpuSurface,
    geometry: WindowGeometry,
    drawable_geometry: WindowGeometry,
    content_zoom: f32,
    maximum_size: Option<(f64, f64)>,
    input: WinitInputAdapter,
    input_source: WindowInputSource,
    presentation: ResolvedWindowPresentation,
    settings: WindowDescriptor,
    #[cfg(not(target_os = "android"))]
    accessibility: Option<HostedAccessibility>,
    accessibility_pending: PendingAccessibility,
    size_move: LiveSizeMove,
    pen: nana_window::PenInput,
    aspect_conform_pending: bool,
    level: WindowLevel,
    requested_level: WindowLevel,
    presented: std::cell::Cell<bool>,
    mode: Option<WindowModeState>,
    pending_fullscreen: Option<FullscreenRequest>,
    native_controls_visible: bool,
    native_controls: Option<nana_ui_runtime::StableNodeId>,
    native_controls_box: std::cell::Cell<Option<nana_ui_runtime::LayoutBox>>,
    skip_taskbar: bool,
    skip_taskbar_report: Option<Result<(), crate::WindowError>>,
    pointer_presence: presence::PointerPresence,
    observed_passthrough_pointer: Option<(f32, f32)>,
    surface_handoff: crate::presentation::SurfaceHandoff,
    monitor: Option<u128>,
    shadow: nana_window::shadow::WindowShadowState,
    shadow_body: Option<ShadowBody>,
    shadow_reported: nana_window::shadow::ShadowWork,
    interaction: nana_window::interaction::WindowInteractionState,
    interaction_regions: nana_ui_runtime::WindowInteractionRegionBuilder,
    interaction_native_snapshot: Arc<nana_ui_platform::InputRegionSnapshot>,
    underlay: Option<nana_window::Underlay>,
    auxiliary: Option<nana_window::AuxiliaryWindow>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ShadowBody {
    revision: (u64, u64),
    size: (f32, f32),
    visible: bool,
    shape: nana_ui_platform::WindowVisualShape,
}

impl Drop for WindowContext {
    fn drop(&mut self) {
        self.shadow.release();
        self.interaction = Default::default();
        self.underlay = None;
        self.auxiliary = None;
        clear_system_material(self.surface.window().as_ref());
    }
}

struct WindowManager<Program: RuntimeProgram> {
    program: Program,
    embedded: bool,
    shutting_down: bool,
    wake_deadline: Option<Instant>,
    host_work: Arc<schedule::HostWorkWake>,
    host_work_deadline: Option<Instant>,
    windows: crate::WindowService,
    window_requests: Option<Receiver<crate::window_service::Request>>,
    next_window_id: u64,
    browsers: HashMap<(WindowId, String), browser::HostedBrowser>,
    web_surfaces: HashMap<String, web_surface::HostedWebSurface>,
    graphics: crate::HostedGpuShared,
    gpu_backend_policy: crate::GpuBackendPolicy,
    non_client: nana_window::NonClientRenderingStrategy,
    composition: crate::presentation::CompositionAvailability,
    painter: Option<SceneWgpuPainter>,
    retained_url_cache: Option<crate::scene_paint::UrlTextureCache>,
    native_renderers:
        HashMap<nana_gpu::GpuTextureFormat, Arc<crate::native_content::NativeContentRenderer>>,
    text: NanaTextShaper,
    proxy: EventLoopProxy,
    message_tx: Sender<Program::Message>,
    messages: Receiver<Program::Message>,
    file_dialogs: dialogs::FileDialogs,
    color_samplers: color_sampler::ColorSamplers,
    tasks: SyncSender<Task<Program::Message>>,
    animation_clock: RuntimeAnimationClock,
    frame_schedules: HashMap<WindowId, crate::runtime_host::FrameSchedule>,
    window_outputs: HashMap<WindowId, crate::output::window_output::WindowOutputState>,
    producer_frames: production::WindowProducerCache,
    direct_output_traces: HashMap<WindowId, crate::output::OutputTraceSnapshot>,
    last_presentation_work: HashMap<WindowId, crate::PresentationWork>,
    damage_trackers: HashMap<WindowId, damage::WindowDamageTracker>,
    texture_subscriptions: HashMap<
        WindowId,
        (
            u64,
            Arc<[nana_ui_runtime::TextureSlotName]>,
            crate::TextureSubscription,
        ),
    >,
    texture_redraws: Arc<Mutex<HashSet<WindowId>>>,
    startup_painter: Option<(
        crate::ScenePresentationProfile,
        Receiver<startup::StartupPainter>,
    )>,
    pipeline_cache: Option<pipeline_cache::HostPipelineCache>,
    material_redraws: Arc<std::sync::atomic::AtomicBool>,
    material_waker: Option<Arc<dyn Fn() + Send + Sync>>,
    image_targets: Arc<Mutex<HashMap<Arc<str>, HashSet<WindowId>>>>,
    image_window_keys: HashMap<WindowId, (u64, Arc<[Arc<str>]>)>,
    occluded: HashSet<WindowId>,
    native_content: HashMap<WindowId, crate::native_content::NativeContentMirror>,
    native_chrome_writes: std::cell::Cell<usize>,
    window_contexts: HashMap<WindowId, WindowContext>,
    window_ids: HashMap<winit::window::WindowId, WindowId>,
    closing_windows: HashSet<WindowId>,
    next_gpu_retry: Option<Instant>,
    render_suspended: bool,
    last_theme: crate::ThemeAppearance,
    installed_theme: Arc<crate::CompiledTheme>,
    window_themes: HashMap<WindowId, Arc<crate::CompiledTheme>>,
    reduced_motion: bool,
    high_contrast: bool,
    reduced_transparency: bool,
    text_scale: Option<f32>,
    settings: WindowDescriptor,
    chrome: HashMap<WindowId, WindowChromeSession>,
    bind_after_present: HashSet<WindowId>,
    startup_failure: Arc<Mutex<Option<String>>>,
    store: SharedStore,
    program_store: SharedStore,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    live_frame_resize: Option<(WindowId, nana_window::LiveFrameResize)>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    live_frame_move: Option<(WindowId, i16, nana_window::LiveFrameMove)>,
    #[cfg(target_os = "macos")]
    present_transaction_pinned: HashSet<WindowId>,
    #[cfg(target_os = "macos")]
    live_resize_commit: HashMap<WindowId, windows::LiveResizeCoalescer>,
    startup: startup::HostStartup,
    input_generations: HashMap<WindowId, u64>,
    clipboard: nana_ui_platform::SharedClipboardHost,
    cursor_cache: nana_window::cursor::PlatformCursorCache,
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    winit_custom_cursors: HashMap<(u64, u32), winit::cursor::CustomCursor>,
    pending_icons: Option<Receiver<SceneIcons>>,
    notifications: nana_window::NotificationHost,
    last_displays: Vec<nana_ui_platform::DisplayInfo>,
    startup_messages: std::collections::VecDeque<Program::Message>,
}

pub(super) struct WindowChromeSession {
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
