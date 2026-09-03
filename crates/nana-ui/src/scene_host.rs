//! Nana-owned winit + wgpu loop for [`crate::RuntimeProgram`].
//!
//! Paint goes through [`crate::SceneWgpuPainter`].

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nana_ui_core::{AppearanceSettings, RESIZE_HANDLE_SIZE, TITLE_BAR_HEIGHT};
use nana_ui_platform::{
    DisplayBounds, ImeEvent, InputEvent, InputModifiers, PointerPhase, PointerType,
    TextInputPurpose, TextInputRequest, WindowCommand, WindowEvent, WindowGeometry, WindowIcon,
    WindowId, WindowResizeEdge, clamp_position_to_displays, clear_registered_application_icon,
    register_application_icon, window_resize_edge,
};
use nana_ui_runtime::{
    AccessibilityUpdate, AppTitleBar, Entity, FrameworkError, LayoutViewport, StableNodeId, Task,
};
#[cfg(target_os = "macos")]
use nana_window::set_application_icon_png;
use nana_window::{
    Appearance, FallbackColor, FrameResizeEdge, LiveSizeMove, MaterialOutcome,
    apply_hosted_system_material, clear_system_material, prepare_client_chrome,
    resize_custom_frame, suppress_system_caption,
};
use winit::application::ApplicationHandler;
use winit::cursor::CursorIcon;
use winit::data_transfer::{DataTransferId, TypeHint};
use winit::dpi::PhysicalPosition;
use winit::event::{
    ButtonSource, DeviceId, ElementState, MouseButton, MouseScrollDelta, PointerKind,
    PointerSource, TabletToolKind, WindowEvent as WinitWindowEvent,
};
use winit::event_loop::{
    ActiveEventLoop, AsyncRequestSerial, ControlFlow, DndAction, EventLoop, EventLoopProxy,
};
use winit::icon::{Icon, RgbaIcon};
use winit::keyboard::ModifiersState;
use winit::monitor::Fullscreen;
#[cfg(target_os = "macos")]
use winit::platform::macos::{WindowAttributesMacOS, WindowExtMacOS};
#[cfg(target_os = "windows")]
use winit::platform::windows::{CornerPreference, WindowAttributesWindows, WindowExtWindows};
#[cfg(target_os = "windows")]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{
    ImeCapabilities, ImeEnableRequest, ImeHint, ImePurpose, ImeRequest, ImeRequestData,
    ImeRequestError, ImeSurroundingText,
};

#[cfg(not(target_os = "android"))]
use crate::accessibility::HostedAccessibility;
use crate::nana_text::NanaTextShaper;
use crate::runtime_host::{
    HostFailure, ImeSurroundingSnapshot, RuntimeProgram, RuntimeProgramContext,
    RuntimeProgramUpdate, RuntimeRedraw, RuntimeWindowSettings, gated_runtime_window_update,
    runtime_ime_surrounding, runtime_text_input_request,
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
const MAX_PROGRAM_DISPATCHES: usize = 32;
const TASK_QUEUE_CAPACITY: usize = 256;
const TASK_WORKERS: usize = 4;

/// Run a [`RuntimeProgram`] on the Nana Scene host.
pub fn run_runtime_scene<Program: RuntimeProgram>(
    settings: RuntimeWindowSettings,
) -> Result<(), HostedRunError> {
    let event_loop = EventLoop::new().map_err(HostedRunError::EventLoop)?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let (message_tx, message_rx) = mpsc::channel();
    let startup_failure = Arc::new(Mutex::new(None));
    let runner = SceneRunner::<Program>::Loading {
        proxy: event_loop.create_proxy(),
        message_tx,
        message_rx,
        settings,
        startup_failure: Arc::clone(&startup_failure),
    };
    event_loop
        .run_app(runner)
        .map_err(HostedRunError::EventLoop)?;
    match startup_failure.lock().ok().and_then(|guard| guard.clone()) {
        Some(message) => Err(HostedRunError::Startup(message)),
        None => Ok(()),
    }
}

enum SceneRunner<Program: RuntimeProgram> {
    Loading {
        proxy: EventLoopProxy,
        message_tx: Sender<Program::Message>,
        message_rx: Receiver<Program::Message>,
        settings: RuntimeWindowSettings,
        startup_failure: Arc<Mutex<Option<String>>>,
    },
    Ready(Box<SceneReady<Program>>),
    Finished {
        startup_failure: Arc<Mutex<Option<String>>>,
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
    size_move: LiveSizeMove,
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
    proxy: EventLoopProxy,
    message_tx: Sender<Program::Message>,
    messages: Receiver<Program::Message>,
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
    next_gpu_retry: Option<Instant>,
    render_suspended: bool,
    last_theme: crate::ThemeMode,
    last_material_mode: nana_window::MaterialEffect,
    settings: RuntimeWindowSettings,
    ime: HashMap<WindowId, AppliedIme>,
    chrome: HashMap<WindowId, WindowChromeSession>,
    bind_after_present: HashSet<WindowId>,
    startup_failure: Arc<Mutex<Option<String>>>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    live_frame_resize: Option<(WindowId, nana_window::LiveFrameResize)>,
    #[cfg(target_os = "macos")]
    present_transaction_pinned: HashSet<WindowId>,
    size_move: LiveSizeMove,
}

struct WindowChromeSession {
    state: WindowChromeState,
    drag: TitleBarDragTracker,
}

#[derive(Debug, Clone, PartialEq)]
struct AppliedIme {
    request: TextInputRequest,
    surrounding: Option<ImeSurroundingSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
enum ImeApply {
    None,
    Disable,
    Enable {
        capabilities: ImeCapabilities,
        data: ImeRequestData,
    },
    Replace {
        capabilities: ImeCapabilities,
        data: ImeRequestData,
    },
    Update(ImeRequestData),
}

fn ime_capabilities(request: &TextInputRequest, has_surrounding: bool) -> ImeCapabilities {
    if !request.enabled {
        return ImeCapabilities::new();
    }
    let mut capabilities = ImeCapabilities::new().with_hint_and_purpose();
    if request.cursor_area.is_some() {
        capabilities = capabilities.with_cursor_area();
    }
    if has_surrounding {
        capabilities = capabilities.with_surrounding_text();
    }
    capabilities
}

fn ime_request_data(
    request: TextInputRequest,
    surrounding: Option<ImeSurroundingText>,
) -> ImeRequestData {
    let purpose = match request.purpose {
        TextInputPurpose::Normal => ImePurpose::Normal,
        TextInputPurpose::Password => ImePurpose::Password,
        TextInputPurpose::Terminal => ImePurpose::Terminal,
    };
    let mut data = ImeRequestData::default().with_hint_and_purpose(ImeHint::NONE, purpose);
    if let Some(cursor) = request.cursor_area {
        data = data.with_cursor_area(
            winit::dpi::LogicalPosition::new(cursor.x, cursor.y + cursor.height).into(),
            winit::dpi::LogicalSize::new(cursor.width.max(1.0), cursor.height.max(1.0)).into(),
        );
    }
    if let Some(surrounding) = surrounding {
        data = data.with_surrounding_text(surrounding);
    }
    data
}

fn ime_apply(
    previous: Option<&TextInputRequest>,
    previous_surrounding: bool,
    next: TextInputRequest,
    surrounding: Option<ImeSurroundingText>,
) -> ImeApply {
    let was_enabled = previous.is_some_and(|request| request.enabled);
    if !next.enabled {
        return if was_enabled {
            ImeApply::Disable
        } else {
            ImeApply::None
        };
    }
    let has_surrounding = surrounding.is_some();
    let capabilities = ime_capabilities(&next, has_surrounding);
    let data = ime_request_data(next, surrounding);
    if !was_enabled {
        return ImeApply::Enable { capabilities, data };
    }
    let previous_capabilities = previous
        .map(|request| ime_capabilities(request, previous_surrounding))
        .unwrap_or_default();
    if previous_capabilities != capabilities {
        ImeApply::Replace { capabilities, data }
    } else {
        ImeApply::Update(data)
    }
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
    fn fail(&mut self, event_loop: &dyn ActiveEventLoop, message: impl Into<String>) {
        let slot = match self {
            Self::Loading {
                startup_failure, ..
            }
            | Self::Finished { startup_failure } => Arc::clone(startup_failure),
            Self::Ready(ready) => Arc::clone(&ready.startup_failure),
        };
        if let Ok(mut guard) = slot.lock() {
            *guard = Some(message.into());
        }
        *self = Self::Finished {
            startup_failure: slot,
        };
        event_loop.exit();
    }
}

impl<Program: RuntimeProgram> ApplicationHandler for SceneRunner<Program> {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !matches!(self, Self::Loading { .. }) {
            return;
        }
        let Self::Loading {
            proxy,
            message_tx,
            message_rx,
            settings,
            startup_failure,
        } = std::mem::replace(
            self,
            Self::Finished {
                startup_failure: Arc::new(Mutex::new(None)),
            },
        )
        else {
            unreachable!("checked Loading");
        };
        match initialize::<Program>(
            event_loop,
            proxy,
            message_tx,
            message_rx,
            settings,
            Arc::clone(&startup_failure),
        ) {
            Ok(ready) => *self = Self::Ready(Box::new(ready)),
            Err(error) => {
                *self = Self::Finished { startup_failure };
                self.fail(event_loop, error);
            }
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        let Self::Ready(ready) = self else {
            return;
        };
        while let Ok(message) = ready.messages.try_recv() {
            ready.process_message(event_loop, message);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
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

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        let Self::Ready(ready) = self else {
            return;
        };
        ready.about_to_wait(event_loop);
    }
}

fn initialize<Program: RuntimeProgram>(
    event_loop: &dyn ActiveEventLoop,
    proxy: EventLoopProxy,
    message_tx: Sender<Program::Message>,
    message_rx: Receiver<Program::Message>,
    settings: RuntimeWindowSettings,
    startup_failure: Arc<Mutex<Option<String>>>,
) -> Result<SceneReady<Program>, String> {
    let window: Arc<dyn winit::window::Window> = Arc::from(
        event_loop
            .create_window(
                scene_window_attributes(&settings, &scene_display_bounds(event_loop))
                    .with_visible(false),
            )
            .map_err(|error| format!("failed to create scene window: {error}"))?,
    );
    apply_scene_window_icon(window.as_ref(), settings.icon.as_ref(), true);
    if !settings.system_caption {
        let _ = prepare_client_chrome(window.as_ref(), f64::from(TITLE_BAR_HEIGHT));
        if settings.transparent {
            let _ = suppress_system_caption(window.as_ref());
        }
    }
    let mut last_theme = crate::ThemeMode::default();
    let mut last_material_mode = nana_window::MaterialEffect::Solid;
    let mut material = apply_window_surface(
        window.as_ref(),
        last_theme,
        settings.transparent,
        last_material_mode,
        AppearanceSettings::DEFAULT_BACKDROP_OPACITY,
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
    let tasks = spawn_task_workers(message_tx.clone(), proxy.clone());
    let geometry = window_geometry(graphics.window().as_ref());
    let context = program_context(
        message_tx.clone(),
        proxy.clone(),
        &graphics,
        WindowId::PRIMARY,
        geometry,
        tasks.clone(),
        material,
        graphics.alpha_mode(),
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
        program.appearance_backdrop_opacity(),
    );
    graphics
        .apply_alpha_mode(window_wants_transparent_surface(
            settings.transparent,
            last_material_mode,
        ))
        .map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "android"))]
    let accessibility = {
        Some(HostedAccessibility::new(
            Arc::clone(graphics.window()),
            accessibility_world_generation(&program, WindowId::PRIMARY),
            accessibility_snapshot(&program, WindowId::PRIMARY),
            true,
            window.scale_factor() as f32,
        ))
    };
    let mut window_ids = HashMap::new();
    window_ids.insert(window.id(), WindowId::PRIMARY);
    let animation_clock = RuntimeAnimationClock::now();
    let mut ready = SceneReady {
        program,
        graphics,
        painters,
        text: NanaTextShaper::default(),
        proxy,
        message_tx,
        messages: message_rx,
        tasks,
        geometry,
        animation_clock,
        default_scene_gpu_renderers,
        #[cfg(not(target_os = "android"))]
        accessibility,
        accessibility_pending: None,
        input: InputTracker::default(),
        material,
        auxiliary: HashMap::new(),
        window_ids,
        next_gpu_retry: None,
        render_suspended: false,
        last_theme,
        last_material_mode,
        settings,
        ime: HashMap::new(),
        chrome: HashMap::new(),
        bind_after_present: HashSet::new(),
        startup_failure,
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        live_frame_resize: None,
        #[cfg(target_os = "macos")]
        present_transaction_pinned: HashSet::new(),
        size_move: LiveSizeMove::install(window.as_ref())?,
    };
    ready
        .program
        .sync_animation_clock(ready.animation_clock.epoch());
    ready.prepare_window_chrome(WindowId::PRIMARY, ready.geometry.maximized);
    let update = ready.program.window_event(
        WindowEvent::Ready {
            id: WindowId::PRIMARY,
            geometry: ready.geometry,
        },
        &ready.context(),
    );
    ready.apply_update(event_loop, update, None);
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
            self.message_tx.clone(),
            self.proxy.clone(),
            &self.graphics,
            id,
            self.geometry_of(id),
            self.tasks.clone(),
            self.material_of(id),
            self.alpha_mode_of(id),
        )
    }

    fn process_message(&mut self, event_loop: &dyn ActiveEventLoop, message: Program::Message) {
        self.bind_after_present.insert(WindowId::PRIMARY);
        let update = self.program.update(message, &self.context());
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
    }

    fn handle_window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
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
            self.apply_update(event_loop, update, None);
            if event_loop.exiting() {
                return;
            }
        }
        if let Some(modal) = self.active_modal_child(id)
            && !allows_modal_parent_event(&event)
        {
            self.focus_window(modal);
            return;
        }
        if let WinitWindowEvent::ModifiersChanged(modifiers) = &event {
            self.input_mut(id).modifiers = modifiers.state();
        }
        if let WinitWindowEvent::PointerMoved { position, .. }
        | WinitWindowEvent::PointerEntered { position, .. } = &event
        {
            let scale = self.scale_factor(id);
            let point = position.to_logical::<f32>(f64::from(scale));
            self.input_mut(id).cursor = (point.x, point.y);
        }
        if let Some(input) = self.normalized_input(id, &event) {
            if self.consume_frame_resize(event_loop, id, &input) {
                return;
            }
            let disposition = self.dispatch_input(event_loop, id, input);
            if matches!(
                &event,
                WinitWindowEvent::PointerMoved { .. } | WinitWindowEvent::PointerEntered { .. }
            ) {
                self.sync_window_cursor(id);
            }
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
            WinitWindowEvent::SurfaceResized(_) | WinitWindowEvent::ScaleFactorChanged { .. } => {
                let geometry_changed = self.sync_geometry(id);
                #[cfg(target_os = "macos")]
                let native_live_resize = self.sync_native_live_resize_presents(id);
                #[cfg(not(target_os = "macos"))]
                let native_live_resize = false;
                self.forward_window_event(event_loop, id, &event);
                // Native macOS drags repaint through winit's live-resize
                // hook, and a custom chrome drag paints its steps in-stack;
                // both would only duplicate the per-step frame here.
                if geometry_changed && !native_live_resize {
                    self.request_redraw(id);
                }
            }
            WinitWindowEvent::Occluded(_) => {
                self.forward_window_event(event_loop, id, &event);
            }
            WinitWindowEvent::Focused(focused) => {
                if !*focused {
                    self.input_mut(id).clear_pointers();
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    self.end_live_frame_resize(id);
                }
                self.forward_window_event(event_loop, id, &event);
                self.apply_ime_request(id);
            }
            WinitWindowEvent::Ime(ime) => {
                self.handle_ime(event_loop, id, platform_ime_event(ime.clone()))
            }
            WinitWindowEvent::DragEntered { .. }
            | WinitWindowEvent::DragPosition { .. }
            | WinitWindowEvent::DragDropped { .. }
            | WinitWindowEvent::DragLeft { .. }
            | WinitWindowEvent::DataTransferReceived { .. } => {
                if let Some(window_event) = self.handle_file_dnd(event_loop, id, &event) {
                    let update = self
                        .program
                        .window_event(window_event, &self.context_for(id));
                    self.apply_update(event_loop, update, None);
                }
            }
            _ => {}
        }
    }

    fn forward_window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        event: &WinitWindowEvent,
    ) {
        if let Some(window_event) = platform_window_event(event, id, self.geometry_of(id)) {
            let update = self
                .program
                .window_event(window_event, &self.context_for(id));
            self.apply_update(event_loop, update, None);
        }
    }

    fn handle_ime(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId, event: ImeEvent) {
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
        self.apply_update(event_loop, update, None);
        self.apply_ime_request(id);
    }

    fn handle_file_dnd(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        event: &WinitWindowEvent,
    ) -> Option<WindowEvent> {
        let scale = self.scale_factor(id);
        match event {
            WinitWindowEvent::DragEntered {
                id: transfer,
                position,
            } => {
                if let Some(position) = position {
                    self.input_mut(id).set_cursor_physical(*position, scale);
                }
                if !dnd_advertises_files(event_loop, *transfer) {
                    return None;
                }
                let _ = event_loop.set_valid_dnd_actions(*transfer, &[DndAction::Copy]);
                let serial = event_loop
                    .fetch_data_transfer(*transfer, &TypeHint::UriList)
                    .ok();
                self.input_mut(id).begin_file_drag(*transfer, serial);
                self.input_mut(id).map_file_window_event(event, id)
            }
            WinitWindowEvent::DragPosition { position, .. } => {
                self.input_mut(id).set_cursor_physical(*position, scale);
                self.input_mut(id).map_file_window_event(event, id)
            }
            WinitWindowEvent::DragDropped { id: transfer, .. } => {
                if !self.input_mut(id).pending_file_paths.is_empty() {
                    return self.input_mut(id).map_file_window_event(event, id);
                }
                match event_loop.fetch_data_transfer(*transfer, &TypeHint::UriList) {
                    Ok(serial) => {
                        self.input_mut(id).wait_for_drop_data(*transfer, serial);
                        None
                    }
                    Err(_) => self.input_mut(id).map_file_window_event(event, id),
                }
            }
            WinitWindowEvent::DragLeft { .. } => {
                self.input_mut(id).map_file_window_event(event, id)
            }
            WinitWindowEvent::DataTransferReceived {
                id: transfer,
                serial,
                value,
            } => {
                if !self.input_mut(id).accepts_dnd_serial(*transfer, *serial) {
                    return None;
                }
                match value.try_as_file_paths() {
                    Ok(paths) => self.input_mut(id).ingest_file_paths(*transfer, paths, id),
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Deadlock
                        ) =>
                    {
                        None
                    }
                    Err(_) => None,
                }
            }
            _ => None,
        }
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        let now = Instant::now();
        #[cfg(target_os = "macos")]
        self.unpin_idle_present_transactions();
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

    fn drain_program_messages(&mut self, id: WindowId) -> RuntimeProgramUpdate {
        let mut update = RuntimeProgramUpdate::default();
        for _ in 0..MAX_PROGRAM_DISPATCHES {
            let queued = self
                .program
                .document_mut(id)
                .map(|document| document.context_mut().take_program_messages())
                .unwrap_or_default();
            if queued.is_empty() {
                break;
            }
            self.bind_after_present.insert(id);
            for boxed in queued {
                let Ok(message) = boxed.downcast::<Program::Message>() else {
                    continue;
                };
                update = update.merge(self.program.update(*message, &self.context_for(id)));
            }
        }
        update
    }

    fn drain_all_program_messages(&mut self) -> RuntimeProgramUpdate {
        let mut update = RuntimeProgramUpdate::default();
        for id in self.known_window_ids() {
            update = update.merge(self.drain_program_messages(id));
        }
        update
    }

    fn wake(&mut self, event_loop: &dyn ActiveEventLoop, now: Instant) {
        let mut update = self.drain_all_program_messages();
        update = update.merge(self.program.wake(now, &self.context()));
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
        update = update.merge(self.drain_all_program_messages());
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
    }

    fn redraw(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId) {
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
        let queued = self.drain_program_messages(id);
        self.apply_update(event_loop, queued, Some(id));
        if event_loop.exiting() || self.render_suspended {
            return;
        }
        self.resize_window(id);
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
        let mut update = self
            .program
            .window_frame_presented(id, &self.context_for(id));
        if self.bind_after_present.remove(&id) {
            update = update.merge(self.program.bind_window(id, &self.context_for(id)));
        }
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
        #[cfg(not(target_os = "android"))]
        if !self.is_live_resize(id) {
            self.synchronize_accessibility(id);
        }
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

    fn apply_update(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        update: RuntimeProgramUpdate,
        painting: Option<WindowId>,
    ) {
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
            if painting == Some(id) {
                continue;
            }
            self.request_redraw(id);
        }
    }

    fn apply_window_command(&mut self, event_loop: &dyn ActiveEventLoop, command: WindowCommand) {
        let known = self.known_window_ids();
        match route_window_command(&command, &known) {
            RoutedWindowCommand::Ignore => {}
            RoutedWindowCommand::Open(id) => {
                let WindowCommand::Open { settings, .. } = command else {
                    return;
                };
                if let Ok(event) = self.open_window(event_loop, id, settings) {
                    let update = self.program.window_event(event, &self.context_for(id));
                    self.program
                        .sync_animation_clock(self.animation_clock.epoch());
                    self.apply_update(event_loop, update, None);
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
                    window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
                }
            }
            RoutedWindowCommand::SetSimpleFullscreen(id) => {
                let WindowCommand::SetSimpleFullscreen { fullscreen, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    #[cfg(target_os = "macos")]
                    window.set_simple_fullscreen(fullscreen);
                    #[cfg(not(target_os = "macos"))]
                    window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
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
                    self.apply_update(event_loop, update, None);
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
            RoutedWindowCommand::SetIcon(id) => {
                let WindowCommand::SetIcon { icon, .. } = command else {
                    return;
                };
                if let Some(window) = self.window(id) {
                    apply_scene_window_icon(
                        window.as_ref(),
                        icon.as_ref(),
                        id == WindowId::PRIMARY,
                    );
                }
            }
            RoutedWindowCommand::SetApplicationIcon => {
                let WindowCommand::SetApplicationIcon { icon } = command else {
                    return;
                };
                match icon {
                    Some(icon) => register_application_icon(icon),
                    None => clear_registered_application_icon(),
                }
                for id in self.known_window_ids() {
                    if let Some(window) = self.window(id) {
                        apply_scene_window_icon(window.as_ref(), None, id == WindowId::PRIMARY);
                    }
                }
                apply_application_icon(&nana_app_icon::resolved_application_icon(None));
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
        event_loop: &dyn ActiveEventLoop,
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
        let attributes = scene_aux_window_attributes(
            &settings,
            parent.as_deref(),
            &scene_display_bounds(event_loop),
        )?;
        let window: Arc<dyn winit::window::Window> = Arc::from(
            event_loop
                .create_window(attributes)
                .map_err(|error| error.to_string())?,
        );
        apply_scene_window_icon(
            window.as_ref(),
            settings.icon.as_ref(),
            id == WindowId::PRIMARY,
        );
        if !settings.system_caption {
            let _ = prepare_client_chrome(window.as_ref(), f64::from(TITLE_BAR_HEIGHT));
            if settings.transparent {
                let _ = suppress_system_caption(window.as_ref());
            }
        }
        let material = apply_window_surface(
            window.as_ref(),
            self.last_theme,
            settings.transparent,
            self.last_material_mode,
            self.program.appearance_backdrop_opacity(),
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
        #[cfg(not(target_os = "android"))]
        let accessibility = {
            Some(HostedAccessibility::new(
                Arc::clone(&window),
                accessibility_world_generation(&self.program, id),
                accessibility_snapshot(&self.program, id),
                true,
                window.scale_factor() as f32,
            ))
        };
        let geometry = window_geometry(window.as_ref());
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
                size_move: LiveSizeMove::install(window.as_ref())?,
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

    fn close_window(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId) {
        if id == WindowId::PRIMARY {
            return;
        }
        self.chrome.remove(&id);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some((_, live)) = self
            .live_frame_resize
            .take_if(|(session, _)| *session == id)
        {
            if let Some(window) = self.window(id) {
                live.end(window.as_ref());
            }
        }
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
            self.ime.remove(&id);
            drop(host);
            let update = self
                .program
                .window_event(WindowEvent::Closed { id }, &self.context_for(id));
            self.apply_update(event_loop, update, None);
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
        let _ = window.request_surface_size(winit::dpi::Size::Logical(
            winit::dpi::LogicalSize::new(f64::from(size.0.max(1.0)), f64::from(size.1.max(1.0))),
        ));
    }

    fn active_modal_child(&self, parent: WindowId) -> Option<WindowId> {
        self.auxiliary.iter().find_map(|(id, host)| {
            (host.settings.modal && host.settings.parent == Some(parent)).then_some(*id)
        })
    }

    fn recover_device(&mut self, event_loop: &dyn ActiveEventLoop) {
        let window = Arc::clone(self.graphics.window());
        let _ = apply_window_surface(
            window.as_ref(),
            self.last_theme,
            self.settings.transparent,
            self.last_material_mode,
            self.program.appearance_backdrop_opacity(),
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
                        self.program.appearance_backdrop_opacity(),
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
                    self.apply_update(event_loop, update, None);
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
            self.program.appearance_backdrop_opacity(),
        );
        let mut alpha_error = self
            .graphics
            .apply_alpha_mode(window_wants_transparent_surface(
                self.settings.transparent,
                self.last_material_mode,
            ))
            .err();
        for host in self.auxiliary.values_mut() {
            clear_system_material(host.surface.window().as_ref());
            host.material = apply_window_surface(
                host.surface.window().as_ref(),
                self.last_theme,
                host.settings.transparent,
                self.last_material_mode,
                self.program.appearance_backdrop_opacity(),
            );
            let want_transparent = window_wants_transparent_surface(
                host.settings.transparent,
                self.last_material_mode,
            );
            if let Err(error) = self
                .graphics
                .apply_surface_alpha_mode(&mut host.surface, want_transparent)
            {
                alpha_error = Some(error);
            }
        }
        if let Some(error) = alpha_error {
            self.suspend_rendering(error);
        }
    }

    /// Refreshes the cached window geometry from the live window state and
    /// reports whether it moved.
    fn sync_geometry(&mut self, id: WindowId) -> bool {
        let previous = self.geometry_of(id);
        if id == WindowId::PRIMARY {
            self.geometry = window_geometry(self.graphics.window().as_ref());
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            host.geometry = window_geometry(host.surface.window().as_ref());
        }
        let changed = self.geometry_of(id) != previous;
        let maximized = self.geometry_of(id).maximized;
        if let Some(session) = self.chrome.get_mut(&id) {
            session.state.update(WindowChromeEvent::MaximizedChanged {
                window: id,
                maximized,
            });
        }
        self.sync_title_bar_maximized(id, maximized);
        changed
    }

    /// Pins transaction presents while the OS's own frame-resize gesture is
    /// moving the window, and reports whether that gesture is active.
    #[cfg(target_os = "macos")]
    fn sync_native_live_resize_presents(&mut self, id: WindowId) -> bool {
        let active = self
            .window(id)
            .is_some_and(|window| nana_window::native_live_resize_active(window.as_ref()));
        if active {
            self.pin_present_transaction(id);
        }
        active
    }

    #[cfg(target_os = "macos")]
    fn pin_present_transaction(&mut self, id: WindowId) {
        if self.present_transaction_pinned.contains(&id) {
            return;
        }
        if let Some(window) = self.window(id)
            && nana_window::set_present_transaction(window.as_ref(), true)
        {
            self.present_transaction_pinned.insert(id);
        }
    }

    /// Releases transaction presents once their resize gesture is over; the
    /// pinned mode serializes every present with a Core Animation commit and
    /// costs latency in steady-state frames.
    #[cfg(target_os = "macos")]
    fn unpin_idle_present_transactions(&mut self) {
        let pinned: Vec<WindowId> = self.present_transaction_pinned.iter().copied().collect();
        for id in pinned {
            let Some(window) = self.window(id) else {
                self.present_transaction_pinned.remove(&id);
                continue;
            };
            if nana_window::native_live_resize_active(window.as_ref()) || self.is_live_resize(id) {
                continue;
            }
            nana_window::set_present_transaction(window.as_ref(), false);
            self.present_transaction_pinned.remove(&id);
        }
    }

    fn resize_window(&mut self, id: WindowId) {
        let live = self.is_live_resize(id);
        if id == WindowId::PRIMARY {
            self.graphics.prepare_frame(live);
        } else if let Some(host) = self.auxiliary.get_mut(&id) {
            self.graphics.prepare_surface_frame(&mut host.surface, live);
        }
    }

    fn is_live_resize(&self, id: WindowId) -> bool {
        if self.size_move_active(id) {
            return true;
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            return self
                .live_frame_resize
                .as_ref()
                .is_some_and(|(session, _)| *session == id);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        false
    }

    fn size_move_active(&self, id: WindowId) -> bool {
        if id == WindowId::PRIMARY {
            self.size_move.is_active()
        } else {
            self.auxiliary
                .get(&id)
                .is_some_and(|host| host.size_move.is_active())
        }
    }

    fn sync_window_cursor(&mut self, id: WindowId) {
        if !self
            .input_mut(id)
            .begin_cursor_sync(std::time::Instant::now())
        {
            return;
        }
        let cursor = self.input_of(id).cursor;
        let frame_edge = self.frame_resize_edge_at(id, cursor.0, cursor.1);
        let (handle, text_field) = self.program.document(id).map_or((None, false), |document| {
            let context = document.context();
            let document_id = document.document();
            let handle = context
                .split_handle_near(document_id, cursor.0, cursor.1)
                .or_else(|| context.dock_handle_near(document_id, cursor.0, cursor.1))
                .or_else(|| context.workspace_handle_near(document_id, cursor.0, cursor.1))
                .and_then(|handle| context.world().layout_box(handle))
                .map(|bounds| (bounds.width, bounds.height));
            let text_field = context
                .pointer_target(document_id, cursor.0, cursor.1)
                .is_some_and(|node| context.world().text_input(node).is_some());
            (handle, text_field)
        });
        if let Some(window) = self.window(id) {
            window.set_cursor(scene_cursor_icon(frame_edge, handle, text_field).into());
        }
    }

    #[cfg_attr(
        not(any(target_os = "macos", target_os = "windows")),
        allow(unused_variables)
    )]
    fn consume_frame_resize(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        input: &InputEvent,
    ) -> bool {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some((session, live)) = self.live_frame_resize
            && session == id
        {
            // The pinned winit win32 proc synthesizes `PointerLeft` from a
            // client-rect bounds check even while the drag is captured, so a
            // fast drag crossing the window edge arrives as `Cancel` and must
            // not end the session; only Up, a fresh primary press (lost Up),
            // or focus loss does.
            match input {
                InputEvent::Pointer {
                    phase: PointerPhase::Move,
                    ..
                } => {
                    if let Some(window) = self.window(id) {
                        let _ = live.update(window.as_ref());
                    }
                    // `setFrame` from inside this pointer dispatch leaves
                    // winit's `SurfaceResized` queued for the next run-loop
                    // pass, and a redraw that waits for it lets the compositor
                    // composite the moved frame with the old drawable
                    // stretched. Sync geometry and paint in this stack, like
                    // the native live-resize path already does.
                    self.sync_geometry(id);
                    self.redraw(event_loop, id);
                    self.sync_window_cursor(id);
                    return true;
                }
                InputEvent::Pointer {
                    phase: PointerPhase::Cancel,
                    ..
                } => {
                    self.sync_window_cursor(id);
                    return true;
                }
                InputEvent::Pointer {
                    phase: PointerPhase::Up,
                    ..
                } => {
                    self.end_live_frame_resize(id);
                    self.request_redraw(id);
                    self.sync_window_cursor(id);
                    return true;
                }
                InputEvent::Pointer {
                    phase: PointerPhase::Down,
                    button: 0,
                    is_primary: true,
                    ..
                } => self.end_live_frame_resize(id),
                _ => {}
            }
        }
        let InputEvent::Pointer {
            phase: PointerPhase::Down,
            button: 0,
            is_primary: true,
            x,
            y,
            ..
        } = input
        else {
            return false;
        };
        let Some(edge) = self.frame_resize_edge_at(id, *x, *y) else {
            return false;
        };
        self.start_frame_resize(id, edge);
        true
    }

    /// Ends the live frame resize for `id` if one is running, releasing the
    /// mouse capture.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn end_live_frame_resize(&mut self, id: WindowId) {
        if let Some((_, live)) = self
            .live_frame_resize
            .take_if(|(session, _)| *session == id)
        {
            if let Some(window) = self.window(id) {
                live.end(window.as_ref());
            }
        }
    }

    fn start_frame_resize(&mut self, id: WindowId, edge: WindowResizeEdge) {
        let Some(window) = self.window(id).cloned() else {
            return;
        };
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            if let Some(live) =
                nana_window::LiveFrameResize::begin(window.as_ref(), frame_resize_edge(edge))
            {
                self.live_frame_resize = Some((id, live));
                // Apply the live present policy before the first moved frame.
                // The mode switch reconfigures the swapchain; paying it on the
                // gesture's first redraw would stall exactly the frame the
                // window starts following the pointer. The native size-move
                // path does not need this: its ENTER hook forces a repaint
                // before the first size change.
                self.resize_window(id);
                #[cfg(target_os = "macos")]
                self.pin_present_transaction(id);
                return;
            }
        }
        resize_scene_window(window.as_ref(), edge);
    }

    fn frame_resize_edge_at(&self, id: WindowId, x: f32, y: f32) -> Option<WindowResizeEdge> {
        let fullscreen = self
            .window(id)
            .is_some_and(|window| window.fullscreen().is_some());
        frame_resize_edge_for(
            self.settings_of(id),
            &self.geometry_of(id),
            fullscreen,
            x,
            y,
        )
    }

    fn settings_of(&self, id: WindowId) -> &RuntimeWindowSettings {
        if id == WindowId::PRIMARY {
            &self.settings
        } else {
            self.auxiliary
                .get(&id)
                .map(|host| &host.settings)
                .unwrap_or(&self.settings)
        }
    }

    fn scale_factor(&self, id: WindowId) -> f32 {
        self.window(id)
            .map(|window| normalized_scale_factor(window.scale_factor() as f32))
            .unwrap_or(1.0)
    }

    fn apply_ime_request(&mut self, id: WindowId) {
        let request = resolved_scene_ime_request(self.program.document(id));
        let surrounding = self.program.document(id).and_then(runtime_ime_surrounding);
        let previous = self.ime.get(&id);
        if previous
            .is_some_and(|applied| applied.request == request && applied.surrounding == surrounding)
        {
            return;
        }
        let Some(window) = self.window(id).cloned() else {
            return;
        };
        // Follow the focused editable field, not NSWindow key status.
        // Gating on has_focus() disables IME while the SCIM candidate panel is
        // key, and also races automation that activates then types immediately.
        let ime_text = surrounding.as_ref().and_then(|snapshot| {
            ImeSurroundingText::new(snapshot.text.clone(), snapshot.cursor, snapshot.anchor).ok()
        });
        apply_text_input_request(
            window.as_ref(),
            ime_apply(
                previous.map(|applied| &applied.request),
                previous.is_some_and(|applied| applied.surrounding.is_some()),
                request,
                ime_text,
            ),
        );
        self.ime.insert(
            id,
            AppliedIme {
                request,
                surrounding,
            },
        );
    }

    fn normalized_input(&mut self, id: WindowId, event: &WinitWindowEvent) -> Option<InputEvent> {
        let scale = self.scale_factor(id);
        let origin = self
            .window(id)
            .and_then(|window| window_screen_origin(window.as_ref()));
        self.input_mut(id).map(event, scale, origin)
    }

    fn dispatch_input(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        input: InputEvent,
    ) -> nana_ui_platform::InputDisposition {
        let now = self.animation_clock.runtime_time(Instant::now());
        let disposition = match self
            .program
            .document_mut(id)
            .map(|document| {
                let document_id = document.document();
                RuntimeInputAdapter::default().dispatch_with_shaper(
                    document.context_mut(),
                    document_id,
                    &input,
                    now,
                    Some(&mut self.text),
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
        // still delivers input_event so Gallery can drain leftover host input and
        // Vue can emit JS. Leftover winit handling stays gated by the caller.
        // Program messages stay queued until the next frame so navigation
        // coalesces and does not run inside the pointer handler.
        let pointer_hit = input_pointer_hit(self.program.document(id), &input);
        let program_input = self.program.input_event_routed(
            id,
            &input,
            pointer_hit,
            &self.context_for(id),
        );
        if let Err(error) = &program_input {
            self.program.host_failure(HostFailure::InputHandler {
                window: id,
                error: error.to_string(),
            });
        }
        let mut update = scene_runtime_input_update(disposition, id, program_input);
        if self
            .program
            .document(id)
            .is_some_and(|document| document.context().has_program_messages())
        {
            update = update.merge(RuntimeProgramUpdate::redraw(id));
        }
        let update = self.merge_title_bar_chrome(id, chrome_action, update);
        self.sync_appearance();
        self.apply_update(event_loop, update, None);
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

    fn known_window_ids(&self) -> Vec<WindowId> {
        let mut ids = vec![WindowId::PRIMARY];
        ids.extend(self.auxiliary.keys().copied());
        ids
    }

    fn window(&self, id: WindowId) -> Option<&Arc<dyn winit::window::Window>> {
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

    fn alpha_mode_of(&self, id: WindowId) -> wgpu::CompositeAlphaMode {
        if id == WindowId::PRIMARY {
            self.graphics.alpha_mode()
        } else {
            self.auxiliary
                .get(&id)
                .map(|host| host.surface.alpha_mode())
                .unwrap_or_else(|| self.graphics.alpha_mode())
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
    message_tx: Sender<Message>,
    proxy: EventLoopProxy,
    graphics: &HostedGpuContext,
    id: WindowId,
    geometry: WindowGeometry,
    tasks: SyncSender<Task<Message>>,
    material: MaterialOutcome,
    surface_alpha_mode: wgpu::CompositeAlphaMode,
) -> RuntimeProgramContext<Message> {
    RuntimeProgramContext::new(
        id,
        geometry,
        graphics.resources(),
        material,
        surface_alpha_mode,
        Arc::new(move |message| {
            if message_tx.send(message).is_ok() {
                proxy.wake_up();
            }
        }),
        tasks,
    )
}

fn spawn_task_workers<Message: Send + 'static>(
    message_tx: Sender<Message>,
    proxy: EventLoopProxy,
) -> SyncSender<Task<Message>> {
    let (sender, receiver) = std::sync::mpsc::sync_channel::<Task<Message>>(TASK_QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    for _ in 0..TASK_WORKERS {
        let receiver = Arc::clone(&receiver);
        let message_tx = message_tx.clone();
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
                if message_tx.send(message).is_err() {
                    return;
                }
                proxy.wake_up();
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
    window: &dyn winit::window::Window,
    theme: crate::ThemeMode,
    requested: crate::MaterialEffect,
    backdrop_opacity: f32,
) -> MaterialOutcome {
    let appearance = match theme {
        crate::ThemeMode::Dark => Appearance::Dark,
        crate::ThemeMode::Light => Appearance::Light,
    };
    let (red, green, blue, _) = theme.palette().background.to_u8_rgba();
    let alpha = (AppearanceSettings::clamp_backdrop_opacity(backdrop_opacity) * 255.0 + 0.5) as u8;
    apply_hosted_system_material(
        window,
        requested,
        appearance,
        FallbackColor::rgba(red, green, blue, alpha),
    )
}

fn apply_window_transparency(window: &dyn winit::window::Window, requested: crate::MaterialEffect) {
    window.set_transparent(requested.wants_transparent_surface());
}

fn apply_window_surface(
    window: &dyn winit::window::Window,
    theme: crate::ThemeMode,
    settings_transparent: bool,
    appearance: crate::MaterialEffect,
    backdrop_opacity: f32,
) -> MaterialOutcome {
    let requested = window_surface_effect(settings_transparent, appearance);
    let material = apply_scene_material(window, theme, requested, backdrop_opacity);
    apply_window_transparency(window, requested);
    material
}

fn drag_scene_window(window: &dyn winit::window::Window) {
    if nana_window::drag_custom_title_bar(window) {
        return;
    }
    let _ = window.drag_window();
}

fn resize_scene_window(window: &dyn winit::window::Window, edge: WindowResizeEdge) {
    if resize_custom_frame(window, frame_resize_edge(edge)) {
        return;
    }
    let _ = window.drag_resize_window(match edge {
        WindowResizeEdge::North => winit::window::ResizeDirection::North,
        WindowResizeEdge::South => winit::window::ResizeDirection::South,
        WindowResizeEdge::East => winit::window::ResizeDirection::East,
        WindowResizeEdge::West => winit::window::ResizeDirection::West,
        WindowResizeEdge::NorthEast => winit::window::ResizeDirection::NorthEast,
        WindowResizeEdge::NorthWest => winit::window::ResizeDirection::NorthWest,
        WindowResizeEdge::SouthEast => winit::window::ResizeDirection::SouthEast,
        WindowResizeEdge::SouthWest => winit::window::ResizeDirection::SouthWest,
    });
}

fn frame_resize_edge(edge: WindowResizeEdge) -> FrameResizeEdge {
    match edge {
        WindowResizeEdge::North => FrameResizeEdge::North,
        WindowResizeEdge::South => FrameResizeEdge::South,
        WindowResizeEdge::East => FrameResizeEdge::East,
        WindowResizeEdge::West => FrameResizeEdge::West,
        WindowResizeEdge::NorthEast => FrameResizeEdge::NorthEast,
        WindowResizeEdge::NorthWest => FrameResizeEdge::NorthWest,
        WindowResizeEdge::SouthEast => FrameResizeEdge::SouthEast,
        WindowResizeEdge::SouthWest => FrameResizeEdge::SouthWest,
    }
}

fn frame_resize_edge_for(
    settings: &RuntimeWindowSettings,
    geometry: &WindowGeometry,
    fullscreen: bool,
    x: f32,
    y: f32,
) -> Option<WindowResizeEdge> {
    if settings.system_caption || !settings.resizable || geometry.maximized || fullscreen {
        return None;
    }
    window_resize_edge(geometry.logical_size, x, y, RESIZE_HANDLE_SIZE)
}

fn scene_cursor_icon(
    frame_edge: Option<WindowResizeEdge>,
    handle: Option<(f32, f32)>,
    text_field: bool,
) -> CursorIcon {
    match frame_edge {
        Some(WindowResizeEdge::East | WindowResizeEdge::West) => CursorIcon::EwResize,
        Some(WindowResizeEdge::North | WindowResizeEdge::South) => CursorIcon::NsResize,
        Some(WindowResizeEdge::NorthEast | WindowResizeEdge::SouthWest) => CursorIcon::NeswResize,
        Some(WindowResizeEdge::NorthWest | WindowResizeEdge::SouthEast) => CursorIcon::NwseResize,
        None => match handle {
            Some((width, height)) => {
                if width <= height {
                    CursorIcon::EwResize
                } else {
                    CursorIcon::NsResize
                }
            }
            None if text_field => CursorIcon::Text,
            None => CursorIcon::Default,
        },
    }
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
    if material.wants_transparent_surface() {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let color = theme.palette().background;
    [color.r, color.g, color.b, color.a]
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

fn enable_ime(
    window: &dyn winit::window::Window,
    capabilities: ImeCapabilities,
    data: ImeRequestData,
) {
    let Some(enable) = ImeEnableRequest::new(capabilities, data.clone()) else {
        return;
    };
    if window.request_ime_update(ImeRequest::Enable(enable)) == Err(ImeRequestError::AlreadyEnabled)
    {
        let _ = window.request_ime_update(ImeRequest::Update(data));
    }
}

fn apply_text_input_request(window: &dyn winit::window::Window, apply: ImeApply) {
    match apply {
        ImeApply::None => {}
        ImeApply::Disable => {
            let _ = window.request_ime_update(ImeRequest::Disable);
        }
        ImeApply::Enable { capabilities, data } => enable_ime(window, capabilities, data),
        ImeApply::Replace { capabilities, data } => {
            let _ = window.request_ime_update(ImeRequest::Disable);
            enable_ime(window, capabilities, data);
        }
        ImeApply::Update(data) => {
            let _ = window.request_ime_update(ImeRequest::Update(data));
        }
    }
}

fn scene_window_attributes(
    settings: &RuntimeWindowSettings,
    displays: &[DisplayBounds],
) -> winit::window::WindowAttributes {
    let mut attributes = winit::window::WindowAttributes::default()
        .with_title(settings.title.clone())
        .with_transparent(settings.transparent)
        .with_resizable(settings.resizable)
        .with_window_level(window_level(settings.always_on_top))
        .with_surface_size(winit::dpi::LogicalSize::new(
            settings.initial_size.0,
            settings.initial_size.1,
        ))
        .with_min_surface_size(winit::dpi::LogicalSize::new(
            settings.minimum_size.0,
            settings.minimum_size.1,
        ))
        .with_maximized(settings.maximized);
    if let Some((x, y)) = settings.initial_position {
        let (x, y) = clamp_position_to_displays((x, y), settings.initial_size, displays);
        attributes = attributes.with_position(winit::dpi::LogicalPosition::new(x, y));
    }
    if let Some(icon) = winit_icon(&resolved_scene_icon(settings.icon.as_ref())) {
        attributes = attributes.with_window_icon(Some(icon));
    }

    apply_scene_window_chrome(attributes, settings)
}

/// Live display bounds in the global logical coordinate space, matching the
/// coordinate space of `WindowSettings::initial_position`.
fn scene_display_bounds(event_loop: &dyn ActiveEventLoop) -> Vec<DisplayBounds> {
    event_loop
        .available_monitors()
        .filter_map(|monitor| {
            let position = monitor.position()?;
            let size = monitor.current_video_mode()?.size();
            let scale = monitor.scale_factor();
            if !scale.is_finite() || scale <= 0.0 {
                return None;
            }
            Some(DisplayBounds {
                position: (f64::from(position.x) / scale, f64::from(position.y) / scale),
                size: (
                    f64::from(size.width) / scale,
                    f64::from(size.height) / scale,
                ),
            })
        })
        .collect()
}

fn resolved_scene_icon(per_window: Option<&WindowIcon>) -> WindowIcon {
    nana_app_icon::resolved_application_icon(per_window)
}

fn winit_icon(icon: &WindowIcon) -> Option<Icon> {
    RgbaIcon::new(icon.rgba.clone(), icon.width, icon.height)
        .ok()
        .map(Icon::from)
}

fn apply_scene_window_icon(
    window: &dyn winit::window::Window,
    per_window: Option<&WindowIcon>,
    apply_app_icon: bool,
) {
    let icon = resolved_scene_icon(per_window);
    // winit 的 Win32 后端把共享的 RGBA 缓冲原地 R/B 翻转成 BGRA;同一 Icon
    // 转换第二次会把颜色换回去,所以每个入口都拿到独立缓冲,恰好转换一次。
    window.set_window_icon(winit_icon(&icon));
    #[cfg(target_os = "windows")]
    window.set_taskbar_icon(winit_icon(&icon));
    if apply_app_icon {
        apply_application_icon(&icon);
    }
}

fn apply_application_icon(icon: &WindowIcon) {
    #[cfg(target_os = "macos")]
    {
        let icon = nana_app_icon::with_system_grid(icon);
        if let Ok(png) = nana_app_icon::encode_png(icon.width, icon.height, &icon.rgba) {
            set_application_icon_png(&png);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = icon;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(all(target_os = "macos", not(test)), allow(dead_code))]
struct WindowsSceneChrome {
    decorations: bool,
    undecorated_shadow: bool,
    no_redirection_bitmap: bool,
}

#[cfg_attr(all(target_os = "macos", not(test)), allow(dead_code))]
fn windows_scene_chrome(system_caption: bool, transparent: bool) -> WindowsSceneChrome {
    WindowsSceneChrome {
        decorations: system_caption,
        undecorated_shadow: !system_caption && !transparent,
        no_redirection_bitmap: transparent,
    }
}

fn apply_scene_window_chrome(
    attributes: winit::window::WindowAttributes,
    settings: &RuntimeWindowSettings,
) -> winit::window::WindowAttributes {
    #[cfg(target_os = "macos")]
    {
        if settings.system_caption {
            return attributes.with_decorations(true);
        }
        attributes
            .with_decorations(true)
            .with_platform_attributes(Box::new(
                WindowAttributesMacOS::default()
                    .with_titlebar_transparent(true)
                    .with_fullsize_content_view(true)
                    .with_title_hidden(true)
                    .with_movable_by_window_background(false),
            ))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let chrome = windows_scene_chrome(settings.system_caption, settings.transparent);
        let attributes = attributes.with_decorations(chrome.decorations);
        #[cfg(target_os = "windows")]
        let attributes = {
            let mut win = WindowAttributesWindows::default()
                .with_no_redirection_bitmap(chrome.no_redirection_bitmap)
                .with_undecorated_shadow(chrome.undecorated_shadow);
            if !chrome.decorations {
                win = win.with_corner_preference(CornerPreference::Round);
            }
            if let Some(icon) = winit_icon(&resolved_scene_icon(settings.icon.as_ref())) {
                win = win.with_taskbar_icon(Some(icon));
            }
            attributes.with_platform_attributes(Box::new(win))
        };
        attributes
    }
}

fn scene_aux_window_attributes(
    settings: &RuntimeWindowSettings,
    parent: Option<&dyn winit::window::Window>,
    displays: &[DisplayBounds],
) -> Result<winit::window::WindowAttributes, String> {
    let attributes = scene_window_attributes(settings, displays).with_visible(false);
    #[cfg(target_os = "windows")]
    let attributes = if settings.modal {
        let parent = parent.ok_or_else(|| "modal window requires a parent".to_string())?;
        let handle = parent
            .window_handle()
            .map_err(|error| format!("failed to acquire modal owner handle: {error}"))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("Windows modal owner is not an HWND".into());
        };
        {
            let chrome = windows_scene_chrome(settings.system_caption, settings.transparent);
            let mut win = WindowAttributesWindows::default()
                .with_no_redirection_bitmap(chrome.no_redirection_bitmap)
                .with_undecorated_shadow(chrome.undecorated_shadow)
                .with_owner_window(handle.hwnd.get() as _);
            if !chrome.decorations {
                win = win.with_corner_preference(CornerPreference::Round);
            }
            if let Some(icon) = winit_icon(&resolved_scene_icon(settings.icon.as_ref())) {
                win = win.with_taskbar_icon(Some(icon));
            }
            attributes.with_platform_attributes(Box::new(win))
        }
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
            | WinitWindowEvent::SurfaceResized(_)
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
    SetSimpleFullscreen(WindowId),
    SetMinimized(WindowId),
    SetMaximized(WindowId),
    SetAlwaysOnTop(WindowId),
    SetIcon(WindowId),
    SetApplicationIcon,
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
        WindowCommand::SetSimpleFullscreen { id, .. } if known(*id) => {
            RoutedWindowCommand::SetSimpleFullscreen(*id)
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
        WindowCommand::SetIcon { id, .. } if known(*id) => RoutedWindowCommand::SetIcon(*id),
        WindowCommand::SetApplicationIcon { .. } => RoutedWindowCommand::SetApplicationIcon,
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

/// Topmost interactive node under the pointer for pointer and wheel events;
/// `None` for every other event.
fn input_pointer_hit(
    document: Option<&nana_ui_scene::RuntimeDocument>,
    event: &InputEvent,
) -> Option<StableNodeId> {
    match event {
        InputEvent::Pointer { x, y, .. } | InputEvent::Wheel { x, y, .. } => document
            .and_then(|document| {
                document
                    .context()
                    .world()
                    .hit_test(document.document(), *x, *y)
            }),
        _ => None,
    }
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

fn window_geometry(window: &dyn winit::window::Window) -> WindowGeometry {
    let scale_factor = normalized_scale_factor(window.scale_factor() as f32);
    let physical_size = window.surface_size();
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
        maximized: geometry_maximized(window),
    }
}

/// macOS 的 winit `is_maximized` 底层是 `is_zoomed`:窗口 mask 为 borderless(进入
/// 全屏后)时它靠临时改回 Titled|Resizable 再回滚来查询,查询本身会触发 resize 事件,
/// 在 resize 事件处理路径中调用就形成死循环。全屏(原生或 simple)语义上不
/// maximized,直接短路;`simple_fullscreen()` 是纯状态读,无副作用。
#[cfg(target_os = "macos")]
fn geometry_maximized(window: &dyn winit::window::Window) -> bool {
    !(window.fullscreen().is_some() || WindowExtMacOS::simple_fullscreen(window))
}

#[cfg(not(target_os = "macos"))]
fn geometry_maximized(window: &dyn winit::window::Window) -> bool {
    window.is_maximized()
}

fn window_screen_origin(window: &dyn winit::window::Window) -> Option<(f32, f32)> {
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
        meta: value.meta_key(),
        shift: value.shift_key(),
    }
}

fn dnd_advertises_files(event_loop: &dyn ActiveEventLoop, transfer: DataTransferId) -> bool {
    event_loop
        .data_transfer(transfer)
        .map(|transfer| transfer.has_type(&TypeHint::UriList))
        .unwrap_or(true)
}

fn platform_ime_event(ime: winit::event::Ime) -> ImeEvent {
    match ime {
        winit::event::Ime::Enabled => ImeEvent::Enabled,
        winit::event::Ime::Disabled => ImeEvent::Disabled,
        winit::event::Ime::Preedit(text, selection) => ImeEvent::Preedit { text, selection },
        winit::event::Ime::Commit(text) => ImeEvent::Commit(text),
        winit::event::Ime::DeleteSurrounding {
            before_bytes,
            after_bytes,
        } => ImeEvent::DeleteSurrounding {
            before_bytes,
            after_bytes,
        },
        _ => ImeEvent::Disabled,
    }
}

fn mouse_button_code(button: MouseButton) -> i16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        other => other as u8 as i16,
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

struct MappedPointer {
    pointer_id: u64,
    pointer_type: PointerType,
    is_primary: bool,
    pressure: Option<f32>,
    tangential_pressure: f32,
    tilt_x: i16,
    tilt_y: i16,
    twist: u16,
}

fn tablet_pointer_id(device_id: Option<DeviceId>, kind: TabletToolKind) -> u64 {
    let device = device_id
        .map(|id| id.into_raw().unsigned_abs())
        .unwrap_or(0);
    let kind_index = u64::from(kind != TabletToolKind::Pen);
    1000 + device.saturating_mul(2) + kind_index
}

fn mapped_pointer(
    pointer_id: u64,
    pointer_type: PointerType,
    primary: bool,
    pressure: Option<f32>,
) -> MappedPointer {
    MappedPointer {
        pointer_id,
        pointer_type,
        is_primary: primary,
        pressure,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
    }
}

fn map_tablet(
    kind: TabletToolKind,
    data: &winit::event::TabletToolData,
    primary: bool,
    device_id: Option<DeviceId>,
) -> MappedPointer {
    let tilt = data.clone().tilt();
    let angle = data.clone().angle();
    MappedPointer {
        pointer_id: tablet_pointer_id(device_id, kind),
        pointer_type: PointerType::Pen,
        is_primary: primary,
        pressure: data
            .force
            .as_ref()
            .map(|force| force.normalized(angle) as f32),
        tangential_pressure: data.tangential_force.unwrap_or(0.0),
        tilt_x: tilt.map(|tilt| i16::from(tilt.x)).unwrap_or(0),
        tilt_y: tilt.map(|tilt| i16::from(tilt.y)).unwrap_or(0),
        twist: data.twist.unwrap_or(0),
    }
}

fn map_pointer_kind(
    kind: &PointerKind,
    primary: bool,
    device_id: Option<DeviceId>,
) -> MappedPointer {
    match kind {
        PointerKind::Touch(finger_id) => mapped_pointer(
            finger_id.into_raw() as u64 + 2,
            PointerType::Touch,
            primary,
            None,
        ),
        PointerKind::TabletTool(kind) => mapped_pointer(
            tablet_pointer_id(device_id, *kind),
            PointerType::Pen,
            primary,
            None,
        ),
        PointerKind::Mouse | PointerKind::Unknown | _ => {
            mapped_pointer(1, PointerType::Mouse, primary, None)
        }
    }
}

fn map_pointer_source(
    source: &PointerSource,
    primary: bool,
    device_id: Option<DeviceId>,
) -> MappedPointer {
    match source {
        PointerSource::Touch { finger_id, force } => mapped_pointer(
            finger_id.into_raw() as u64 + 2,
            PointerType::Touch,
            primary,
            force.as_ref().map(|force| force.normalized(None) as f32),
        ),
        PointerSource::TabletTool { kind, data } => map_tablet(*kind, data, primary, device_id),
        PointerSource::Mouse | PointerSource::Unknown | _ => {
            map_pointer_kind(&PointerKind::Mouse, primary, device_id)
        }
    }
}

fn map_button_source(
    source: &ButtonSource,
    primary: bool,
    device_id: Option<DeviceId>,
) -> MappedPointer {
    match source {
        ButtonSource::Touch { finger_id, force } => mapped_pointer(
            finger_id.into_raw() as u64 + 2,
            PointerType::Touch,
            primary,
            force.as_ref().map(|force| force.normalized(None) as f32),
        ),
        ButtonSource::TabletTool { kind, data, .. } => map_tablet(*kind, data, primary, device_id),
        ButtonSource::Mouse(_) | ButtonSource::Unknown(_) | _ => {
            map_pointer_kind(&PointerKind::Mouse, primary, device_id)
        }
    }
}

#[derive(Debug, Default)]
struct InputTracker {
    cursor: (f32, f32),
    cursor_sync_last: Option<std::time::Instant>,
    buttons: u16,
    modifiers: ModifiersState,
    active_touches: HashSet<u64>,
    primary_touch: Option<u64>,
    pending_file_paths: Vec<PathBuf>,
    file_drop_emitted: bool,
    pending_dnd: Option<DataTransferId>,
    pending_dnd_serial: Option<AsyncRequestSerial>,
    drop_waiting_for_data: bool,
}

impl InputTracker {
    fn clear_pointers(&mut self) {
        self.buttons = 0;
        self.active_touches.clear();
        self.primary_touch = None;
    }

    fn set_cursor_physical(&mut self, position: PhysicalPosition<f64>, scale: f32) {
        let point = position.to_logical::<f32>(f64::from(scale));
        self.cursor = (point.x, point.y);
    }

    /// Whether a cursor-icon sync may run now; records the sync when true.
    ///
    /// The sync probes split/dock/workspace handles, and each probe walks the
    /// whole document when the pointer is outside every handle slop. Pointer
    /// moves arrive faster than frames, so gate the probe to one per frame
    /// interval; the icon lagging a frame is imperceptible.
    fn begin_cursor_sync(&mut self, now: std::time::Instant) -> bool {
        const CURSOR_SYNC_INTERVAL: std::time::Duration = std::time::Duration::from_millis(8);
        if self
            .cursor_sync_last
            .is_some_and(|last| now.duration_since(last) < CURSOR_SYNC_INTERVAL)
        {
            return false;
        }
        self.cursor_sync_last = Some(now);
        true
    }

    fn pointer_event(
        &self,
        mapped: MappedPointer,
        phase: PointerPhase,
        button: i16,
        buttons: u16,
        activation_click: bool,
        modifiers: InputModifiers,
        screen_origin: Option<(f32, f32)>,
        pressure: Option<f32>,
    ) -> InputEvent {
        let screen = screen_position(screen_origin, self.cursor);
        InputEvent::Pointer {
            phase,
            pointer_id: mapped.pointer_id,
            pointer_type: mapped.pointer_type,
            x: self.cursor.0,
            y: self.cursor.1,
            screen_x: screen.0,
            screen_y: screen.1,
            button,
            buttons,
            pressure: pressure.unwrap_or_else(|| {
                mapped
                    .pressure
                    .unwrap_or(if buttons == 0 { 0.0 } else { 0.5 })
            }),
            tangential_pressure: mapped.tangential_pressure,
            tilt_x: mapped.tilt_x,
            tilt_y: mapped.tilt_y,
            twist: mapped.twist,
            is_primary: mapped.is_primary,
            activation_click,
            modifiers,
        }
    }

    fn begin_file_drag(&mut self, transfer: DataTransferId, serial: Option<AsyncRequestSerial>) {
        self.pending_file_paths.clear();
        self.file_drop_emitted = false;
        self.drop_waiting_for_data = false;
        self.pending_dnd = Some(transfer);
        self.pending_dnd_serial = serial;
    }

    fn wait_for_drop_data(&mut self, transfer: DataTransferId, serial: AsyncRequestSerial) {
        self.pending_dnd = Some(transfer);
        self.pending_dnd_serial = Some(serial);
        self.drop_waiting_for_data = true;
    }

    fn accepts_dnd_serial(&self, transfer: DataTransferId, serial: AsyncRequestSerial) -> bool {
        self.pending_dnd == Some(transfer)
            && self
                .pending_dnd_serial
                .is_none_or(|pending| pending == serial)
    }

    fn ingest_file_paths(
        &mut self,
        transfer: DataTransferId,
        paths: Vec<PathBuf>,
        id: WindowId,
    ) -> Option<WindowEvent> {
        if self.pending_dnd != Some(transfer) {
            return None;
        }
        self.pending_file_paths = paths;
        if self.drop_waiting_for_data {
            if self.file_drop_emitted {
                return None;
            }
            self.file_drop_emitted = true;
            self.drop_waiting_for_data = false;
            self.pending_dnd = None;
            self.pending_dnd_serial = None;
            return Some(WindowEvent::FileDropped {
                id,
                paths: std::mem::take(&mut self.pending_file_paths),
                position: Some(self.cursor),
            });
        }
        Some(WindowEvent::FileHovered {
            id,
            paths: self.pending_file_paths.clone(),
            position: Some(self.cursor),
        })
    }

    fn map(
        &mut self,
        event: &WinitWindowEvent,
        scale: f32,
        screen_origin: Option<(f32, f32)>,
    ) -> Option<InputEvent> {
        let modifiers = platform_input_modifiers(self.modifiers);
        match event {
            WinitWindowEvent::PointerMoved {
                device_id,
                position,
                primary,
                source,
            } => {
                self.set_cursor_physical(*position, scale);
                Some(self.pointer_event(
                    map_pointer_source(source, *primary, *device_id),
                    PointerPhase::Move,
                    -1,
                    self.buttons,
                    false,
                    modifiers,
                    screen_origin,
                    None,
                ))
            }
            WinitWindowEvent::PointerEntered {
                device_id,
                position,
                primary,
                kind,
            } => {
                self.set_cursor_physical(*position, scale);
                Some(self.pointer_event(
                    map_pointer_kind(kind, *primary, *device_id),
                    PointerPhase::Move,
                    -1,
                    self.buttons,
                    false,
                    modifiers,
                    screen_origin,
                    None,
                ))
            }
            WinitWindowEvent::PointerButton {
                device_id,
                state,
                position,
                primary,
                button,
                is_macos_activation_click,
            } => {
                self.set_cursor_physical(*position, scale);
                let mouse = button.clone().mouse_button().unwrap_or(MouseButton::Left);
                let button_code = mouse_button_code(mouse);
                let pressed = *state == ElementState::Pressed;
                let mask = mouse_button_mask(button_code);
                if pressed {
                    self.buttons |= mask;
                } else {
                    self.buttons &= !mask;
                }
                Some(self.pointer_event(
                    map_button_source(button, *primary, *device_id),
                    if pressed {
                        PointerPhase::Down
                    } else {
                        PointerPhase::Up
                    },
                    button_code,
                    self.buttons,
                    *is_macos_activation_click,
                    modifiers,
                    screen_origin,
                    None,
                ))
            }
            WinitWindowEvent::PointerLeft {
                device_id,
                position,
                primary,
                kind,
            } => {
                if let Some(position) = position {
                    self.set_cursor_physical(*position, scale);
                }
                let buttons = std::mem::take(&mut self.buttons);
                Some(self.pointer_event(
                    map_pointer_kind(kind, *primary, *device_id),
                    PointerPhase::Cancel,
                    -1,
                    buttons,
                    false,
                    modifiers,
                    screen_origin,
                    Some(0.0),
                ))
            }
            WinitWindowEvent::MouseWheel { delta, .. } => {
                let (delta_x, delta_y, line_delta) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x, *y, true),
                    MouseScrollDelta::PixelDelta(delta) => (
                        (delta.x / f64::from(scale)) as f32,
                        (delta.y / f64::from(scale)) as f32,
                        false,
                    ),
                    _ => (0.0, 0.0, false),
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
            WinitWindowEvent::DragEntered { id: transfer, .. } => {
                if self.pending_dnd != Some(*transfer) {
                    self.begin_file_drag(*transfer, None);
                }
                Some(WindowEvent::FileHovered {
                    id,
                    paths: self.pending_file_paths.clone(),
                    position: Some(self.cursor),
                })
            }
            WinitWindowEvent::DragPosition { id: transfer, .. } => {
                if self.pending_dnd != Some(*transfer) || self.file_drop_emitted {
                    return None;
                }
                Some(WindowEvent::FileHovered {
                    id,
                    paths: self.pending_file_paths.clone(),
                    position: Some(self.cursor),
                })
            }
            WinitWindowEvent::DragLeft { .. } => {
                self.pending_file_paths.clear();
                self.file_drop_emitted = false;
                self.drop_waiting_for_data = false;
                self.pending_dnd = None;
                self.pending_dnd_serial = None;
                Some(WindowEvent::FileHoverCancelled { id })
            }
            WinitWindowEvent::DragDropped { .. } => {
                if self.file_drop_emitted {
                    return None;
                }
                self.file_drop_emitted = true;
                self.drop_waiting_for_data = false;
                self.pending_dnd = None;
                self.pending_dnd_serial = None;
                Some(WindowEvent::FileDropped {
                    id,
                    paths: std::mem::take(&mut self.pending_file_paths),
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
        WinitWindowEvent::SurfaceResized(_) | WinitWindowEvent::ScaleFactorChanged { .. } => {
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
        DisplayBounds, ImeApply, InputTracker, RoutedWindowCommand, ime_apply,
        input_pointer_hit, invalidate_program_host_textures, mouse_button_code,
        mouse_button_mask, platform_ime_event, platform_input_key, platform_input_modifiers,
        platform_window_event, resolved_scene_ime_request, route_window_command,
        scene_clear_color, scene_runtime_input_update, scene_window_attributes, screen_position,
        should_deliver_program_ime, tablet_pointer_id, window_level, window_surface_effect,
        window_wants_transparent_surface, windows_scene_chrome, windows_to_redraw, winit_icon,
    };
    use crate::{
        HostTexture, HostTextureAlphaMode, HostTextureRegistry, MaterialEffect, MaterialOutcome,
        RuntimeProgramUpdate, RuntimeRedraw, ThemeMode,
    };
    use nana_ui_platform::{
        ImeEvent, InputDisposition, InputEvent, PointerPhase, PointerType, TextInputPurpose,
        TextInputRequest, WindowCommand, WindowEvent, WindowGeometry, WindowIcon, WindowId,
        WindowResizeEdge, WindowSettings,
    };
    #[cfg(not(target_os = "android"))]
    use nana_ui_runtime::{AccessibilityDelta, AccessibilityUpdate, FrameworkError};
    use winit::dpi::PhysicalPosition;
    use winit::event::{
        ButtonSource, DeviceId, ElementState, FingerId, MouseButton, MouseScrollDelta, PointerKind,
        PointerSource, TabletToolData, TabletToolKind, TouchPhase, WindowEvent as WinitWindowEvent,
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
    fn input_pointer_hit_reports_the_topmost_node() {
        use nana_ui_platform::InputModifiers;
        use nana_ui_runtime::{Button, DocumentId, LayoutViewport, MeasureTextShaper};
        use nana_ui_scene::RuntimeDocument;

        let document_id = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document_id);
        let button = runtime
            .context_mut()
            .build(document_id, |ui| ui.child("build", Button::new("Build")))
            .unwrap();
        runtime
            .flush(LayoutViewport::new(320.0, 180.0), &mut MeasureTextShaper)
            .unwrap();
        let layout = runtime
            .context()
            .world()
            .layout_box(button.stable_id())
            .unwrap();

        let wheel = InputEvent::Wheel {
            x: layout.x + layout.width / 2.0,
            y: layout.y + layout.height / 2.0,
            delta_x: 0.0,
            delta_y: 1.0,
            line_delta: true,
            modifiers: InputModifiers::default(),
        };
        assert_eq!(
            input_pointer_hit(Some(&runtime), &wheel),
            Some(button.stable_id())
        );

        let outside = InputEvent::Wheel {
            x: layout.x + layout.width + 40.0,
            y: layout.y + layout.height + 40.0,
            delta_x: 0.0,
            delta_y: -1.0,
            line_delta: true,
            modifiers: InputModifiers::default(),
        };
        assert_eq!(input_pointer_hit(Some(&runtime), &outside), None);

        let keyboard = InputEvent::Keyboard {
            pressed: true,
            key: "Escape".to_string(),
            text: None,
            code: "Escape".to_string(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert_eq!(input_pointer_hit(Some(&runtime), &keyboard), None);
        assert_eq!(input_pointer_hit(None, &wheel), None);
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
        let attributes = scene_window_attributes(&settings, &[]);

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

        let transparent_client =
            windows_scene_chrome(settings.system_caption, settings.transparent);
        assert!(!transparent_client.decorations);
        assert!(!transparent_client.undecorated_shadow);
        assert!(transparent_client.no_redirection_bitmap);

        let opaque_client = windows_scene_chrome(false, false);
        assert!(!opaque_client.decorations);
        assert!(opaque_client.undecorated_shadow);
        assert!(!opaque_client.no_redirection_bitmap);

        settings.system_caption = true;
        let caption = scene_window_attributes(&settings, &[]);
        assert!(caption.decorations);
        let transparent_caption = windows_scene_chrome(true, true);
        assert!(transparent_caption.decorations);
        assert!(!transparent_caption.undecorated_shadow);
        assert!(transparent_caption.no_redirection_bitmap);
        let opaque_caption = windows_scene_chrome(true, false);
        assert!(opaque_caption.decorations);
        assert!(!opaque_caption.no_redirection_bitmap);
    }

    #[test]
    fn window_and_taskbar_icons_convert_independent_buffers() {
        // winit 的 Win32 后端原地翻转共享缓冲;同一 winit Icon 不允许被转换两次,
        // 否则任务栏大图标的 R/B 被换回、蓝色标记显示为橙黄。
        let source = WindowIcon::from_rgba(vec![73; 8 * 8 * 4], 8, 8).expect("valid icon source");
        let window_icon = winit_icon(&source).expect("window icon");
        let taskbar_icon = winit_icon(&source).expect("taskbar icon");
        assert!(
            !std::sync::Arc::ptr_eq(&window_icon.0, &taskbar_icon.0),
            "each applied icon must own its RGBA buffer"
        );
    }

    #[test]
    fn scene_windows_reclamp_offscreen_initial_positions_to_live_displays() {
        let mut settings = WindowSettings::new("Scene");
        settings.initial_size = (888.0, 586.0);
        settings.initial_position = Some((2100.0, 40.0));
        let main = [DisplayBounds {
            position: (0.0, 0.0),
            size: (1920.0, 1080.0),
        }];

        let attributes = scene_window_attributes(&settings, &main);
        assert_eq!(
            attributes.position,
            Some(winit::dpi::Position::Logical(
                winit::dpi::LogicalPosition::new(1032.0, 40.0)
            ))
        );

        let disconnected = [
            main[0],
            DisplayBounds {
                position: (1920.0, 0.0),
                size: (1080.0, 1920.0),
            },
        ];
        let attributes = scene_window_attributes(&settings, &disconnected);
        assert_eq!(
            attributes.position,
            Some(winit::dpi::Position::Logical(
                winit::dpi::LogicalPosition::new(2100.0, 40.0)
            ))
        );

        settings.initial_position = None;
        assert_eq!(scene_window_attributes(&settings, &main).position, None);
    }

    #[test]
    fn native_and_transparent_materials_clear_the_surface_to_zero_alpha() {
        let solid = scene_clear_color(ThemeMode::Dark, MaterialOutcome::chosen_solid());
        assert!(solid[3] > 0.0, "opaque windows keep a readable clear color");
        assert_eq!(
            scene_clear_color(ThemeMode::Dark, MaterialOutcome::transparent()),
            [0.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(
            scene_clear_color(
                ThemeMode::Dark,
                MaterialOutcome::native(MaterialEffect::Mica)
            ),
            [0.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(
            scene_clear_color(
                ThemeMode::Light,
                MaterialOutcome::native(MaterialEffect::Acrylic)
            ),
            [0.0, 0.0, 0.0, 0.0]
        );
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
                | ModifiersState::META,
        );
        assert!(modifiers.control);
        assert!(modifiers.alt);
        assert!(modifiers.shift);
        assert!(modifiers.meta);
    }

    #[test]
    fn cursor_sync_is_throttled_to_one_per_frame_interval() {
        let mut tracker = InputTracker::default();
        assert!(tracker.begin_cursor_sync(std::time::Instant::now()));
        // A second sync inside the frame interval is skipped.
        assert!(!tracker.begin_cursor_sync(std::time::Instant::now()));
        std::thread::sleep(std::time::Duration::from_millis(9));
        assert!(tracker.begin_cursor_sync(std::time::Instant::now()));
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
                &WinitWindowEvent::PointerMoved {
                    device_id: None,
                    position: PhysicalPosition::new(20.0, 40.0),
                    primary: true,
                    source: PointerSource::Mouse,
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
                &WinitWindowEvent::PointerButton {
                    device_id: None,
                    state: ElementState::Pressed,
                    position: PhysicalPosition::new(20.0, 40.0),
                    primary: true,
                    button: ButtonSource::Mouse(MouseButton::Left),
                    is_macos_activation_click: false,
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

        let activation = tracker
            .map(
                &WinitWindowEvent::PointerButton {
                    device_id: None,
                    state: ElementState::Pressed,
                    position: PhysicalPosition::new(20.0, 40.0),
                    primary: true,
                    button: ButtonSource::Mouse(MouseButton::Left),
                    is_macos_activation_click: true,
                },
                2.0,
                Some((100.0, 200.0)),
            )
            .expect("activation down");
        let InputEvent::Pointer {
            activation_click, ..
        } = activation
        else {
            panic!("expected pointer");
        };
        assert!(activation_click);

        let left = tracker
            .map(
                &WinitWindowEvent::PointerLeft {
                    device_id: None,
                    position: None,
                    primary: true,
                    kind: PointerKind::Mouse,
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
    fn pointer_enter_emits_move_without_a_followup_moved() {
        let mut tracker = InputTracker::default();
        let entered = tracker
            .map(
                &WinitWindowEvent::PointerEntered {
                    device_id: None,
                    position: PhysicalPosition::new(20.0, 40.0),
                    primary: true,
                    kind: PointerKind::Mouse,
                },
                2.0,
                Some((100.0, 200.0)),
            )
            .expect("pointer enter");
        let InputEvent::Pointer {
            phase,
            x,
            y,
            screen_x,
            screen_y,
            pointer_type,
            button,
            ..
        } = entered
        else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Move);
        assert_eq!(pointer_type, PointerType::Mouse);
        assert_eq!((x, y), (10.0, 20.0));
        assert_eq!((screen_x, screen_y), (110.0, 220.0));
        assert_eq!(button, -1);
    }

    #[test]
    fn pointer_leave_uses_event_position_when_present() {
        let mut tracker = InputTracker {
            cursor: (1.0, 1.0),
            ..InputTracker::default()
        };
        let left = tracker
            .map(
                &WinitWindowEvent::PointerLeft {
                    device_id: None,
                    position: Some(PhysicalPosition::new(40.0, 80.0)),
                    primary: true,
                    kind: PointerKind::Mouse,
                },
                2.0,
                None,
            )
            .expect("pointer leave");
        let InputEvent::Pointer { phase, x, y, .. } = left else {
            panic!("expected pointer");
        };
        assert_eq!(phase, PointerPhase::Cancel);
        assert_eq!((x, y), (20.0, 40.0));
    }

    #[test]
    fn tablet_pointer_ids_split_device_and_tool_kind() {
        let pen = DeviceId::from_raw(2);
        let other = DeviceId::from_raw(3);
        assert_ne!(
            tablet_pointer_id(Some(pen), TabletToolKind::Pen),
            tablet_pointer_id(Some(pen), TabletToolKind::Eraser)
        );
        assert_ne!(
            tablet_pointer_id(Some(pen), TabletToolKind::Pen),
            tablet_pointer_id(Some(other), TabletToolKind::Pen)
        );
        let mut tracker = InputTracker::default();
        let moved = tracker
            .map(
                &WinitWindowEvent::PointerMoved {
                    device_id: Some(pen),
                    position: PhysicalPosition::new(4.0, 8.0),
                    primary: true,
                    source: PointerSource::TabletTool {
                        kind: TabletToolKind::Eraser,
                        data: TabletToolData::default(),
                    },
                },
                1.0,
                None,
            )
            .expect("pen move");
        let InputEvent::Pointer {
            pointer_id,
            pointer_type,
            ..
        } = moved
        else {
            panic!("expected pointer");
        };
        assert_eq!(pointer_type, PointerType::Pen);
        assert_eq!(
            pointer_id,
            tablet_pointer_id(Some(pen), TabletToolKind::Eraser)
        );
        assert_ne!(pointer_id, 1000);
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
                    device_id: None,
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
                    device_id: None,
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
                &WinitWindowEvent::PointerButton {
                    device_id: None,
                    state: ElementState::Pressed,
                    position: PhysicalPosition::new(4.0, 8.0),
                    primary: true,
                    button: ButtonSource::Touch {
                        finger_id: FingerId::from_raw(3),
                        force: None,
                    },
                    is_macos_activation_click: false,
                },
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
            platform_ime_event(winit::event::Ime::DeleteSurrounding {
                before_bytes: 3,
                after_bytes: 0,
            }),
            ImeEvent::DeleteSurrounding {
                before_bytes: 3,
                after_bytes: 0,
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
        let transfer = winit::data_transfer::DataTransferId::from_raw(1);
        assert!(matches!(
            tracker.map_file_window_event(
                &WinitWindowEvent::DragEntered {
                    id: transfer,
                    position: None,
                },
                WindowId::PRIMARY,
            ),
            Some(WindowEvent::FileHovered { .. })
        ));
        assert!(matches!(
            tracker.map_file_window_event(
                &WinitWindowEvent::DragDropped {
                    id: transfer,
                    proposed_action: None,
                },
                WindowId::PRIMARY,
            ),
            Some(WindowEvent::FileDropped { .. })
        ));
        assert!(
            tracker
                .map_file_window_event(
                    &WinitWindowEvent::DragDropped {
                        id: transfer,
                        proposed_action: None,
                    },
                    WindowId::PRIMARY,
                )
                .is_none()
        );

        let mut cancelled = InputTracker::default();
        cancelled.map_file_window_event(
            &WinitWindowEvent::DragEntered {
                id: transfer,
                position: None,
            },
            WindowId::PRIMARY,
        );
        assert!(matches!(
            cancelled.map_file_window_event(
                &WinitWindowEvent::DragLeft { id: transfer },
                WindowId::PRIMARY,
            ),
            Some(WindowEvent::FileHoverCancelled { .. })
        ));
    }

    #[test]
    fn file_drag_ingests_fetched_paths_before_and_after_drop() {
        let transfer = winit::data_transfer::DataTransferId::from_raw(7);
        let paths = vec![std::path::PathBuf::from("/tmp/nana.txt")];
        let mut hover = InputTracker {
            cursor: (8.0, 16.0),
            ..InputTracker::default()
        };
        hover.begin_file_drag(transfer, None);
        assert_eq!(
            hover.ingest_file_paths(transfer, paths.clone(), WindowId::PRIMARY),
            Some(WindowEvent::FileHovered {
                id: WindowId::PRIMARY,
                paths: paths.clone(),
                position: Some((8.0, 16.0)),
            })
        );
        assert_eq!(
            hover.map_file_window_event(
                &WinitWindowEvent::DragDropped {
                    id: transfer,
                    proposed_action: None,
                },
                WindowId::PRIMARY,
            ),
            Some(WindowEvent::FileDropped {
                id: WindowId::PRIMARY,
                paths: paths.clone(),
                position: Some((8.0, 16.0)),
            })
        );

        let mut delayed = InputTracker::default();
        delayed.wait_for_drop_data(transfer, winit::event_loop::AsyncRequestSerial::get());
        assert_eq!(
            delayed.ingest_file_paths(transfer, paths.clone(), WindowId::PRIMARY),
            Some(WindowEvent::FileDropped {
                id: WindowId::PRIMARY,
                paths,
                position: Some((0.0, 0.0)),
            })
        );
        assert!(
            delayed
                .map_file_window_event(
                    &WinitWindowEvent::DragDropped {
                        id: transfer,
                        proposed_action: None,
                    },
                    WindowId::PRIMARY,
                )
                .is_none()
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

    fn ime_request(
        enabled: bool,
        cursor: Option<(f32, f32, f32, f32)>,
        purpose: TextInputPurpose,
    ) -> TextInputRequest {
        TextInputRequest {
            enabled,
            cursor_area: cursor
                .map(|(x, y, width, height)| nana_ui_core::LogicalRect::new(x, y, width, height)),
            purpose,
        }
    }

    #[test]
    fn ime_apply_enables_once_then_updates_caret() {
        let off = ime_request(false, None, TextInputPurpose::Normal);
        let first = ime_request(
            true,
            Some((10.0, 20.0, 8.0, 16.0)),
            TextInputPurpose::Normal,
        );
        assert!(matches!(
            ime_apply(Some(&off), false, first, None),
            ImeApply::Enable { .. }
        ));

        let moved = ime_request(
            true,
            Some((12.0, 20.0, 8.0, 16.0)),
            TextInputPurpose::Normal,
        );
        assert!(matches!(
            ime_apply(Some(&first), false, moved, None),
            ImeApply::Update(_)
        ));
    }

    #[test]
    fn ime_apply_replaces_when_cursor_area_capability_appears() {
        let without_caret = ime_request(true, None, TextInputPurpose::Normal);
        let with_caret = ime_request(true, Some((4.0, 8.0, 2.0, 12.0)), TextInputPurpose::Normal);
        assert!(matches!(
            ime_apply(Some(&without_caret), false, with_caret, None),
            ImeApply::Replace { .. }
        ));
    }

    #[test]
    fn ime_apply_disables_when_leaving_the_field() {
        let on = ime_request(true, Some((1.0, 2.0, 3.0, 4.0)), TextInputPurpose::Normal);
        let off = ime_request(false, None, TextInputPurpose::Normal);
        assert!(matches!(
            ime_apply(Some(&on), false, off, None),
            ImeApply::Disable
        ));
        assert!(matches!(
            ime_apply(Some(&off), false, off, None),
            ImeApply::None
        ));
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
            route_window_command(
                &WindowCommand::SetIcon {
                    id: tool,
                    icon: None,
                },
                &known
            ),
            RoutedWindowCommand::SetIcon(tool)
        );
        assert_eq!(
            route_window_command(&WindowCommand::SetApplicationIcon { icon: None }, &known),
            RoutedWindowCommand::SetApplicationIcon
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
    fn client_frame_resize_hits_edges_unless_caption_or_maximized() {
        use super::{frame_resize_edge_for, scene_cursor_icon};
        use winit::cursor::CursorIcon;

        let mut settings = WindowSettings::new("Scene");
        let mut geometry = geometry();
        geometry.logical_size = (800.0, 600.0);
        assert_eq!(
            frame_resize_edge_for(&settings, &geometry, false, 2.0, 300.0),
            Some(WindowResizeEdge::West)
        );
        assert_eq!(
            frame_resize_edge_for(&settings, &geometry, false, 400.0, 300.0),
            None
        );
        settings.system_caption = true;
        assert!(frame_resize_edge_for(&settings, &geometry, false, 2.0, 300.0).is_none());
        settings.system_caption = false;
        settings.resizable = false;
        assert!(frame_resize_edge_for(&settings, &geometry, false, 2.0, 300.0).is_none());
        settings.resizable = true;
        geometry.maximized = true;
        assert!(frame_resize_edge_for(&settings, &geometry, false, 2.0, 300.0).is_none());
        geometry.maximized = false;
        assert!(frame_resize_edge_for(&settings, &geometry, true, 2.0, 300.0).is_none());
        assert_eq!(
            scene_cursor_icon(Some(WindowResizeEdge::East), Some((8.0, 200.0)), true),
            CursorIcon::EwResize
        );
        assert_eq!(
            scene_cursor_icon(None, Some((8.0, 200.0)), true),
            CursorIcon::EwResize
        );
        assert_eq!(
            scene_cursor_icon(None, Some((200.0, 8.0)), false),
            CursorIcon::NsResize
        );
        assert_eq!(scene_cursor_icon(None, None, true), CursorIcon::Text);
        assert_eq!(scene_cursor_icon(None, None, false), CursorIcon::Default);
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
