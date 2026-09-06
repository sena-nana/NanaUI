//! Standard Rust application owner. Custom and JS hosts may still implement
//! `RuntimeProgram` directly; application business state stays in `State`.
use crate::{
    FrameDemand, HostTextureRegistry, RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate,
    SceneGpuRendererRegistry, SceneResourceProducerRegistry, ThemeMode,
};
use nana_ui_platform::{WindowEvent, WindowId};
use nana_ui_scene::{DocumentAccessError, RuntimeDocument};
use std::collections::HashMap;

pub struct ApplicationWindow {
    pub document: RuntimeDocument,
    pub textures: HostTextureRegistry,
    pub renderers: Option<SceneGpuRendererRegistry>,
    pub producers: Option<SceneResourceProducerRegistry>,
    pub demand: FrameDemand,
}

impl ApplicationWindow {
    pub fn new() -> Self {
        Self {
            // Each window owns an independent UiWorld. Document identities
            // are local to that world; WindowId handles host routing.
            document: RuntimeDocument::new(nana_ui_runtime::DocumentId::new(1).unwrap()),
            textures: HostTextureRegistry::new(),
            renderers: None,
            producers: None,
            demand: FrameDemand::OnDemand,
        }
    }
}

impl Default for ApplicationWindow {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ApplicationState: Sized + 'static {
    type Message: Send + 'static;
    type Error: std::fmt::Display;
    fn initialize(context: &RuntimeProgramContext<Self::Message>) -> Result<Self, Self::Error>;
    fn build(
        &mut self,
        window: &mut ApplicationWindow,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), Self::Error>;
    fn update(
        &mut self,
        _message: Self::Message,
        _windows: &mut HashMap<WindowId, ApplicationWindow>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }
    fn theme_mode(&self) -> ThemeMode {
        ThemeMode::Dark
    }
    /// Release application-owned state keyed by a window that has closed.
    fn window_closed(&mut self, _id: WindowId) {}
    fn prepare(
        &mut self,
        _window: &mut ApplicationWindow,
        _context: &RuntimeProgramContext<Self::Message>,
    ) {
    }
    fn presented(
        &mut self,
        _window: &mut ApplicationWindow,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }
    fn rebuild_gpu(
        &mut self,
        _windows: &mut HashMap<WindowId, ApplicationWindow>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) {
    }
    fn host_failure(&mut self, failure: crate::HostFailure) {
        eprintln!("NanaUI host failure: {failure:?}");
    }
}

pub struct RuntimeApplication<State: ApplicationState> {
    pub state: State,
    pub windows: HashMap<WindowId, ApplicationWindow>,
}

impl<State: ApplicationState> RuntimeProgram for RuntimeApplication<State> {
    type Message = State::Message;
    type Error = State::Error;
    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let mut state = State::initialize(context)?;
        let mut window = ApplicationWindow::new();
        state.build(&mut window, context)?;
        Ok((
            Self {
                state,
                windows: HashMap::from([(context.window_id(), window)]),
            },
            Vec::new(),
        ))
    }
    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok(self.windows.get(&id).map(|window| f(&window.document)))
    }
    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok(self
            .windows
            .get_mut(&id)
            .map(|window| f(&mut window.document)))
    }
    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.state.update(message, &mut self.windows, context)
    }
    fn theme_mode(&self) -> ThemeMode {
        self.state.theme_mode()
    }
    fn frame_demand(&self, id: WindowId) -> FrameDemand {
        self.windows
            .get(&id)
            .map_or(FrameDemand::OnDemand, |window| window.demand)
    }
    fn host_textures(&self, id: WindowId) -> Option<HostTextureRegistry> {
        self.windows.get(&id).map(|window| window.textures.clone())
    }
    fn scene_gpu_renderers(&self, id: WindowId) -> Option<SceneGpuRendererRegistry> {
        self.windows
            .get(&id)
            .and_then(|window| window.renderers.clone())
    }
    fn scene_resource_producers(&self, id: WindowId) -> Option<SceneResourceProducerRegistry> {
        self.windows
            .get(&id)
            .and_then(|window| window.producers.clone())
    }
    fn prepare_window_frame(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if let Some(window) = self.windows.get_mut(&id) {
            self.state.prepare(window, context);
        }
    }
    fn window_frame_presented(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.windows
            .get_mut(&id)
            .map_or_else(RuntimeProgramUpdate::default, |window| {
                self.state.presented(window, context)
            })
    }
    fn window_event(
        &mut self,
        event: WindowEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::Ready { id, .. } if !self.windows.contains_key(&id) => {
                let mut window = ApplicationWindow::new();
                if let Err(error) = self.state.build(&mut window, context) {
                    eprintln!("NanaUI window build failed: {error}");
                    return RuntimeProgramUpdate {
                        window_commands: vec![nana_ui_platform::WindowCommand::Close(id)],
                        ..Default::default()
                    };
                }
                self.windows.insert(id, window);
                RuntimeProgramUpdate::redraw(id)
            }
            WindowEvent::Closed { id } => {
                self.windows.remove(&id);
                self.state.window_closed(id);
                RuntimeProgramUpdate::default()
            }
            WindowEvent::CloseRequested { id } if id == WindowId::PRIMARY => {
                RuntimeProgramUpdate::exit()
            }
            WindowEvent::CloseRequested { id } => RuntimeProgramUpdate {
                window_commands: vec![nana_ui_platform::WindowCommand::Close(id)],
                ..Default::default()
            },
            _ => RuntimeProgramUpdate::default(),
        }
    }
    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        self.state.rebuild_gpu(&mut self.windows, context);
    }
    fn host_failure(&mut self, failure: crate::HostFailure) {
        self.state.host_failure(failure);
    }
}
