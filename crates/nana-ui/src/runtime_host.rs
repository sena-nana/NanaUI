//! Backend-neutral application contract backed by the existing desktop host.
//!
//! Applications own [`RuntimeDocument`] values and never build an Iced tree.
//! The private adapter below turns each retained [`UiScene`] into the hosted
//! renderer's compatibility element at the final backend boundary.

use std::collections::HashMap;
use std::fmt;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iced::{Element, Size};
use nana_ui_platform::{
    InputDisposition, InputEvent, WindowCommand, WindowEvent, WindowGeometry, WindowId, WindowRole,
    WindowSettings,
};
use nana_ui_runtime::{
    AccessibilityActionRequest, AccessibilityNode, AccessibilityUpdate, AnimationFrame,
    FrameworkError, Task,
};
use nana_ui_scene::RuntimeDocument;

use crate::{
    HostTextureRegistry, HostedGpuResources, HostedProgram, HostedProgramContext,
    HostedProgramUpdate, HostedRedraw, HostedWindowCommand, HostedWindowEvent,
    HostedWindowGeometry, HostedWindowRole, HostedWindowSettings, IcedSceneView,
    RuntimeAnimationClock, RuntimeInputAdapter, ThemeMode, run_hosted,
};

pub use nana_ui_platform::WindowSettings as RuntimeWindowSettings;

/// Host services that are safe to retain or invoke from application code.
/// Native window and Iced identities intentionally do not cross this boundary.
#[derive(Clone)]
pub struct RuntimeProgramContext<Message: Send + 'static> {
    window_id: WindowId,
    geometry: WindowGeometry,
    gpu: HostedGpuResources,
    dispatch: Arc<dyn Fn(Message) + Send + Sync>,
    tasks: SyncSender<Task<Message>>,
}

impl<Message: Send + 'static> RuntimeProgramContext<Message> {
    pub const fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub const fn geometry(&self) -> WindowGeometry {
        self.geometry
    }

    pub fn gpu(&self) -> &HostedGpuResources {
        &self.gpu
    }

    pub fn dispatch(&self, message: Message) {
        (self.dispatch)(message);
    }

    /// Run a Nana Runtime task on host-owned execution infrastructure and
    /// route its completion back through the native event loop.
    pub fn run_task(&self, task: Task<Message>) -> Result<(), RuntimeTaskError> {
        self.tasks.try_send(task).map_err(|error| match error {
            TrySendError::Full(_) => RuntimeTaskError::QueueFull,
            TrySendError::Disconnected(_) => RuntimeTaskError::HostStopped,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeTaskError {
    QueueFull,
    HostStopped,
}

impl fmt::Display for RuntimeTaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::QueueFull => "runtime host task queue is full",
            Self::HostStopped => "runtime host task executor has stopped",
        })
    }
}

impl std::error::Error for RuntimeTaskError {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RuntimeRedraw {
    #[default]
    None,
    Window(WindowId),
    All,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeProgramUpdate {
    pub redraw: RuntimeRedraw,
    pub window_commands: Vec<WindowCommand>,
    pub exit: bool,
}

impl RuntimeProgramUpdate {
    pub const fn redraw(id: WindowId) -> Self {
        Self {
            redraw: RuntimeRedraw::Window(id),
            window_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn redraw_all() -> Self {
        Self {
            redraw: RuntimeRedraw::All,
            window_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn exit() -> Self {
        Self {
            redraw: RuntimeRedraw::None,
            window_commands: Vec::new(),
            exit: true,
        }
    }

    fn merge(mut self, other: Self) -> Self {
        self.exit |= other.exit;
        self.window_commands.extend(other.window_commands);
        self.redraw = match (self.redraw, other.redraw) {
            (RuntimeRedraw::All, _) | (_, RuntimeRedraw::All) => RuntimeRedraw::All,
            (RuntimeRedraw::None, redraw) | (redraw, RuntimeRedraw::None) => redraw,
            (RuntimeRedraw::Window(left), RuntimeRedraw::Window(right)) if left == right => {
                RuntimeRedraw::Window(left)
            }
            _ => RuntimeRedraw::All,
        };
        self
    }
}

/// Canonical retained application contract. Iced remains available only
/// through [`HostedProgram`] for compatibility consumers.
pub trait RuntimeProgram: Sized + 'static {
    type Message: Send + 'static;
    type Error: fmt::Display;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error>;

    fn document(&self, id: WindowId) -> Option<&RuntimeDocument>;
    fn document_mut(&mut self, id: WindowId) -> Option<&mut RuntimeDocument>;

    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate;

    fn theme_mode(&self) -> ThemeMode;

    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        None
    }

    /// Give the application a chance to commit text measurement and layout
    /// writeback before the frame driver drains the document.
    fn prepare_frame(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), FrameworkError> {
        Ok(())
    }

    fn input_event(
        &mut self,
        _id: WindowId,
        _event: &InputEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        Ok(RuntimeProgramUpdate::default())
    }

    fn window_event(
        &mut self,
        _event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    fn animation_frame(
        &mut self,
        _id: WindowId,
        _frame: AnimationFrame,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        Ok(RuntimeProgramUpdate::default())
    }

    fn accessibility_action(
        &mut self,
        id: WindowId,
        request: AccessibilityActionRequest,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let changed = self
            .document_mut(id)
            .map(|document| {
                let document_id = document.document();
                document
                    .context_mut()
                    .apply_accessibility_action(document_id, request)
            })
            .transpose()?
            .unwrap_or(false);
        Ok(if changed {
            RuntimeProgramUpdate::redraw(id)
        } else {
            RuntimeProgramUpdate::default()
        })
    }
}

struct RuntimeHosted<Program: RuntimeProgram> {
    program: Program,
    geometries: HashMap<WindowId, WindowGeometry>,
    accessibility: HashMap<WindowId, AccessibilityUpdate>,
    animation_clock: RuntimeAnimationClock,
    tasks: SyncSender<Task<Program::Message>>,
}

enum RuntimeHostMessage<Message> {
    App(Message),
}

impl<Program: RuntimeProgram> RuntimeHosted<Program> {
    fn context(
        hosted: &HostedProgramContext<RuntimeHostMessage<Program::Message>>,
        id: WindowId,
        geometry: WindowGeometry,
        tasks: SyncSender<Task<Program::Message>>,
    ) -> RuntimeProgramContext<Program::Message> {
        let proxy = hosted.proxy().clone();
        RuntimeProgramContext {
            window_id: id,
            geometry,
            gpu: hosted.gpu().clone(),
            dispatch: Arc::new(move |message| {
                let _ = proxy.send_event(RuntimeHostMessage::App(message));
            }),
            tasks,
        }
    }

    fn hosted_update(update: RuntimeProgramUpdate) -> HostedProgramUpdate {
        HostedProgramUpdate {
            redraw: match update.redraw {
                RuntimeRedraw::None => HostedRedraw::None,
                RuntimeRedraw::Window(WindowId::PRIMARY) => HostedRedraw::Primary,
                RuntimeRedraw::Window(id) => HostedRedraw::Window(id),
                RuntimeRedraw::All => HostedRedraw::All,
            },
            window_commands: update
                .window_commands
                .into_iter()
                .map(hosted_window_command)
                .collect(),
            exit: update.exit,
            ..HostedProgramUpdate::default()
        }
    }

    fn view_for(&self, id: WindowId) -> Element<'static, RuntimeHostMessage<Program::Message>> {
        let document = self
            .program
            .document(id)
            .unwrap_or_else(|| panic!("RuntimeProgram has no document for window {}", id.0));
        let geometry = self.geometries.get(&id).copied().unwrap_or_default();
        IcedSceneView::from_shared(
            document.shared_scene(),
            self.program.host_textures(id),
            Size::new(geometry.logical_size.0, geometry.logical_size.1),
        )
        .unwrap_or_else(|error| panic!("RuntimeProgram produced an unpaintable UiScene: {error}"))
        .into()
    }
}

impl<Program: RuntimeProgram> HostedProgram for RuntimeHosted<Program> {
    type Message = RuntimeHostMessage<Program::Message>;
    type Error = Program::Error;

    fn initialize(
        hosted: &HostedProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let geometry = platform_geometry(hosted.geometry());
        let tasks = runtime_task_workers(hosted.proxy().clone());
        let context = Self::context(hosted, WindowId::PRIMARY, geometry, tasks.clone());
        let (program, messages) = Program::initialize(&context)?;
        Ok((
            Self {
                program,
                geometries: HashMap::from([(WindowId::PRIMARY, geometry)]),
                accessibility: HashMap::new(),
                animation_clock: RuntimeAnimationClock::now(),
                tasks,
            },
            messages.into_iter().map(RuntimeHostMessage::App).collect(),
        ))
    }

    fn update(
        &mut self,
        message: Self::Message,
        hosted: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        let RuntimeHostMessage::App(message) = message;
        let geometry = self
            .geometries
            .get(&WindowId::PRIMARY)
            .copied()
            .unwrap_or_else(|| platform_geometry(hosted.geometry()));
        let context = Self::context(hosted, WindowId::PRIMARY, geometry, self.tasks.clone());
        Self::hosted_update(self.program.update(message, &context))
    }

    fn view(&self, _native_material: bool) -> Element<'static, Self::Message> {
        self.view_for(WindowId::PRIMARY)
    }

    fn view_window(&self, id: WindowId, _native_material: bool) -> Element<'static, Self::Message> {
        self.view_for(id)
    }

    fn theme_mode(&self) -> ThemeMode {
        self.program.theme_mode()
    }

    fn prepare_window_frame(&mut self, id: WindowId, hosted: &HostedProgramContext<Self::Message>) {
        let geometry = self
            .geometries
            .get(&id)
            .copied()
            .unwrap_or_else(|| platform_geometry(hosted.geometry()));
        let context = Self::context(hosted, id, geometry, self.tasks.clone());
        self.program
            .prepare_frame(id, &context)
            .unwrap_or_else(|error| panic!("RuntimeProgram frame preparation failed: {error}"));
        let update = self
            .program
            .document_mut(id)
            .unwrap_or_else(|| panic!("RuntimeProgram has no document for window {}", id.0))
            .flush_with(|_, _| Ok(()))
            .unwrap_or_else(|error| panic!("RuntimeProgram frame did not settle: {error}"));
        if !update.accessibility.updated.is_empty() || !update.accessibility.removed.is_empty() {
            self.accessibility
                .insert(id, AccessibilityUpdate::Delta(update.accessibility));
        }
    }

    fn input_event(
        &mut self,
        id: WindowId,
        event: InputEvent,
        hosted: &HostedProgramContext<Self::Message>,
    ) -> (InputDisposition, HostedProgramUpdate) {
        let geometry = self.geometries.get(&id).copied().unwrap_or_default();
        let context = Self::context(hosted, id, geometry, self.tasks.clone());
        let disposition = self
            .program
            .document_mut(id)
            .map(|document| {
                let document_id = document.document();
                RuntimeInputAdapter::default().dispatch(document.context_mut(), document_id, &event)
            })
            .transpose()
            .unwrap_or_else(|error| panic!("RuntimeProgram input dispatch failed: {error}"))
            .unwrap_or_default();
        let mut update = self
            .program
            .input_event(id, &event, &context)
            .unwrap_or_else(|error| panic!("RuntimeProgram input handler failed: {error}"));
        if disposition.prevent_default {
            update = update.merge(RuntimeProgramUpdate::redraw(id));
        }
        (disposition, Self::hosted_update(update))
    }

    fn window_event(
        &mut self,
        event: HostedWindowEvent,
        hosted: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        let Some(event) = platform_window_event(&event) else {
            return HostedProgramUpdate::default();
        };
        let id = event_window_id(&event);
        if let Some(geometry) = event_geometry(&event) {
            self.geometries.insert(id, geometry);
        }
        if matches!(event, WindowEvent::Closed { .. }) {
            self.geometries.remove(&id);
            self.accessibility.remove(&id);
        }
        let geometry = self.geometries.get(&id).copied().unwrap_or_default();
        let context = Self::context(hosted, id, geometry, self.tasks.clone());
        let ime_changed = if let WindowEvent::Ime { event, .. } = &event {
            self.program
                .document_mut(id)
                .map(|document| {
                    let document_id = document.document();
                    match event {
                        nana_ui_platform::ImeEvent::Enabled => Ok(false),
                        nana_ui_platform::ImeEvent::Disabled => {
                            document.context_mut().clear_ime(document_id)
                        }
                        nana_ui_platform::ImeEvent::Preedit { text, selection } => document
                            .context_mut()
                            .set_ime_preedit(document_id, text.clone(), *selection),
                        nana_ui_platform::ImeEvent::Commit(text) => {
                            document.context_mut().commit_ime(document_id, text)
                        }
                    }
                })
                .transpose()
                .unwrap_or_else(|error| panic!("RuntimeProgram IME dispatch failed: {error}"))
                .unwrap_or(false)
        } else {
            false
        };
        let mut update = self.program.window_event(event, &context);
        if ime_changed {
            update = update.merge(RuntimeProgramUpdate::redraw(id));
        }
        Self::hosted_update(update)
    }

    fn accessibility_snapshot(&self, id: WindowId) -> Vec<AccessibilityNode> {
        self.program
            .document(id)
            .map(|document| {
                document
                    .context()
                    .world()
                    .project_accessibility(document.document())
            })
            .unwrap_or_default()
    }

    fn accessibility_adapter_enabled(&self) -> bool {
        true
    }

    fn accessibility_update(&mut self, id: WindowId) -> Option<AccessibilityUpdate> {
        self.accessibility.remove(&id)
    }

    fn accessibility_actions_enabled(&self) -> bool {
        true
    }

    fn text_input_request(&self, id: WindowId) -> Option<nana_ui_platform::TextInputRequest> {
        let document = self.program.document(id)?;
        let focused = document
            .context()
            .focused_text_input(document.document())
            .map(|(target, _)| target);
        let cursor_area = focused
            .and_then(|target| document.context().world().layout_box(target))
            .map(|layout| {
                nana_ui_core::LogicalRect::new(layout.x, layout.y, layout.width, layout.height)
            });
        Some(nana_ui_platform::TextInputRequest {
            enabled: focused.is_some(),
            cursor_area,
            purpose: nana_ui_platform::TextInputPurpose::Normal,
        })
    }

    fn accessibility_action(
        &mut self,
        id: WindowId,
        request: AccessibilityActionRequest,
        hosted: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        let geometry = self.geometries.get(&id).copied().unwrap_or_default();
        let context = Self::context(hosted, id, geometry, self.tasks.clone());
        Self::hosted_update(
            self.program
                .accessibility_action(id, request, &context)
                .unwrap_or_else(|error| {
                    panic!("RuntimeProgram accessibility action failed: {error}")
                }),
        )
    }

    fn next_wakeup(&self) -> Option<Instant> {
        self.geometries
            .keys()
            .filter_map(|id| {
                self.program
                    .document(*id)
                    .and_then(|document| self.animation_clock.next_wakeup(document.context()))
            })
            .min()
    }

    fn wake(
        &mut self,
        now: Instant,
        hosted: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        let ids = self.geometries.keys().copied().collect::<Vec<_>>();
        let mut update = RuntimeProgramUpdate::default();
        for id in ids {
            let frame = self
                .program
                .document_mut(id)
                .map(|document| self.animation_clock.wake(document.context_mut(), now));
            let Some(frame) = frame else {
                continue;
            };
            let had_samples = !frame.samples.is_empty();
            let geometry = self.geometries.get(&id).copied().unwrap_or_default();
            let context = Self::context(hosted, id, geometry, self.tasks.clone());
            update = update.merge(
                self.program
                    .animation_frame(id, frame, &context)
                    .unwrap_or_else(|error| {
                        panic!("RuntimeProgram animation handler failed: {error}")
                    }),
            );
            if had_samples {
                update = update.merge(RuntimeProgramUpdate::redraw(id));
            }
        }
        Self::hosted_update(update)
    }
}

pub fn run_runtime<Program: RuntimeProgram>(
    settings: WindowSettings,
) -> Result<(), crate::HostedRunError> {
    run_hosted::<RuntimeHosted<Program>>(hosted_window_settings(settings))
}

fn runtime_task_workers<Message: Send + 'static>(
    proxy: iced_winit::winit::event_loop::EventLoopProxy<RuntimeHostMessage<Message>>,
) -> SyncSender<Task<Message>> {
    const TASK_QUEUE_CAPACITY: usize = 256;
    const TASK_WORKERS: usize = 4;
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
                let message = iced_winit::futures::futures::executor::block_on(task.into_future());
                if proxy.send_event(RuntimeHostMessage::App(message)).is_err() {
                    return;
                }
            }
        });
    }
    sender
}

fn hosted_window_settings(settings: WindowSettings) -> HostedWindowSettings {
    let mut hosted = HostedWindowSettings::new(settings.title)
        .initial_size(settings.initial_size.0, settings.initial_size.1)
        .minimum_size(settings.minimum_size.0, settings.minimum_size.1)
        .maximized(settings.maximized)
        .transparent(settings.transparent)
        .always_on_top(settings.always_on_top)
        .resizable(settings.resizable);
    if let Some((x, y)) = settings.initial_position {
        hosted = hosted.initial_position(x, y);
    }
    hosted.role = match settings.role {
        WindowRole::Main => HostedWindowRole::Main,
        WindowRole::Tool => HostedWindowRole::Tool,
    };
    hosted.modal = settings.modal;
    hosted.parent = settings.parent;
    hosted
}

fn hosted_window_command(command: WindowCommand) -> HostedWindowCommand {
    match command {
        WindowCommand::Open { id, settings } => HostedWindowCommand::Open {
            id,
            settings: hosted_window_settings(settings),
        },
        WindowCommand::Close(id) => HostedWindowCommand::Close(id),
        WindowCommand::Move { id, position } => HostedWindowCommand::Move {
            id,
            position: iced::Point::new(position.0, position.1),
        },
        WindowCommand::SetTitle { id, title } => HostedWindowCommand::SetTitle { id, title },
        WindowCommand::SetBounds { id, position, size } => HostedWindowCommand::SetBounds {
            id,
            position: iced::Point::new(position.0, position.1),
            width: size.0,
            height: size.1,
        },
        WindowCommand::SetFullscreen { id, fullscreen } => {
            HostedWindowCommand::SetFullscreen { id, fullscreen }
        }
        WindowCommand::SetMinimized { id, minimized } => {
            HostedWindowCommand::SetMinimized { id, minimized }
        }
        WindowCommand::SetMaximized { id, maximized } => {
            HostedWindowCommand::SetMaximized { id, maximized }
        }
        WindowCommand::SetAlwaysOnTop { id, always_on_top } => {
            HostedWindowCommand::SetAlwaysOnTop { id, always_on_top }
        }
        WindowCommand::Focus(id) => HostedWindowCommand::Focus(id),
    }
}

fn platform_geometry(geometry: HostedWindowGeometry) -> WindowGeometry {
    WindowGeometry {
        physical_position: geometry.physical_position,
        physical_size: (geometry.physical_size.width, geometry.physical_size.height),
        logical_position: geometry.logical_position.map(|point| (point.x, point.y)),
        logical_size: (geometry.logical_size.width, geometry.logical_size.height),
        scale_factor: geometry.scale_factor,
        maximized: geometry.maximized,
    }
}

fn platform_window_event(event: &HostedWindowEvent) -> Option<WindowEvent> {
    Some(match event {
        HostedWindowEvent::Ready { id, geometry, .. } => WindowEvent::Ready {
            id: *id,
            geometry: platform_geometry(*geometry),
        },
        HostedWindowEvent::Resized { id, geometry, .. } => WindowEvent::Resized {
            id: *id,
            geometry: platform_geometry(*geometry),
        },
        HostedWindowEvent::Moved { id, geometry, .. } => WindowEvent::Moved {
            id: *id,
            geometry: platform_geometry(*geometry),
        },
        HostedWindowEvent::VisibilityChanged { id, hidden, .. } => WindowEvent::VisibilityChanged {
            id: *id,
            hidden: *hidden,
        },
        HostedWindowEvent::FocusChanged { id, focused, .. } => WindowEvent::FocusChanged {
            id: *id,
            focused: *focused,
        },
        HostedWindowEvent::Ime { id, event, .. } => WindowEvent::Ime {
            id: *id,
            event: event.clone(),
        },
        HostedWindowEvent::CloseRequested { id, .. } => WindowEvent::CloseRequested { id: *id },
        HostedWindowEvent::Closed { id, .. } => WindowEvent::Closed { id: *id },
        HostedWindowEvent::FileHovered { .. }
        | HostedWindowEvent::FilesHovered { .. }
        | HostedWindowEvent::FileDropped { .. }
        | HostedWindowEvent::FilesDropped { .. }
        | HostedWindowEvent::FileHoverCancelled { .. }
        | HostedWindowEvent::KeyPressed { .. } => return None,
    })
}

fn event_window_id(event: &WindowEvent) -> WindowId {
    match event {
        WindowEvent::Ready { id, .. }
        | WindowEvent::Resized { id, .. }
        | WindowEvent::Moved { id, .. }
        | WindowEvent::VisibilityChanged { id, .. }
        | WindowEvent::FocusChanged { id, .. }
        | WindowEvent::Ime { id, .. }
        | WindowEvent::CloseRequested { id }
        | WindowEvent::Closed { id } => *id,
    }
}

fn event_geometry(event: &WindowEvent) -> Option<WindowGeometry> {
    match event {
        WindowEvent::Ready { geometry, .. }
        | WindowEvent::Resized { geometry, .. }
        | WindowEvent::Moved { geometry, .. } => Some(*geometry),
        _ => None,
    }
}
