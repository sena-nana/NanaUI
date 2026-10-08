//! Standard Rust application owner. Custom and JS hosts may still implement
//! `RuntimeProgram` directly; application business state stays in `State`.
use crate::{
    FrameDemand, HostTextureRegistry, RoutedInput, RuntimeProgram, RuntimeProgramContext,
    RuntimeProgramUpdate, SceneGpuRendererRegistry, SceneResourceProducerRegistry, ThemeAppearance,
};
use nana_ui_core::{CompiledTheme, builtin_theme_arc};
use nana_ui_platform::{WindowEvent, WindowId};
use nana_ui_scene::{DocumentAccessError, RuntimeDocument};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

pub struct ApplicationWindow {
    pub document: RuntimeDocument,
    pub textures: HostTextureRegistry,
    pub renderers: Option<SceneGpuRendererRegistry>,
    pub producers: Option<SceneResourceProducerRegistry>,
    /// Egress for this window's `http(s)` `url(...)` images; `None` refuses them.
    /// Keep one host per window: replacing the `Arc` refetches its images.
    pub fetch_host: Option<nana_ui_platform::SharedFetchHost>,
    pub demand: FrameDemand,
    /// Paint this window a second time into an offscreen output; frames and
    /// state changes arrive at [`ApplicationState::output_frame`] and
    /// [`ApplicationState::output_status`].
    pub output: Option<crate::WindowOutputConfig>,
}

impl ApplicationWindow {
    pub fn new() -> Self {
        Self::with_document(RuntimeDocument::new(Self::DOCUMENT))
    }

    /// A window whose built-in components are created from their Rust types
    /// only ([`nana_ui_runtime::BuiltinComponents::Typed`]).
    pub fn typed() -> Self {
        Self::with_document(RuntimeDocument::typed(Self::DOCUMENT))
    }

    // Each window owns an independent UiWorld. Document identities are local
    // to that world; WindowId handles host routing.
    const DOCUMENT: nana_ui_runtime::DocumentId = nana_ui_runtime::DocumentId::new(1).unwrap();

    /// `S::BUILTINS` is a constant in each instance, so the mode not chosen
    /// is not linked.
    fn for_state<S: ApplicationState>() -> Self {
        match S::BUILTINS {
            nana_ui_runtime::BuiltinComponents::Full => Self::new(),
            nana_ui_runtime::BuiltinComponents::Typed => Self::typed(),
        }
    }

    fn with_document(document: RuntimeDocument) -> Self {
        Self {
            document,
            textures: HostTextureRegistry::new(),
            renderers: None,
            producers: None,
            fetch_host: None,
            demand: FrameDemand::OnDemand,
            output: None,
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
    /// The built-in component machinery each window installs. `Typed` suits
    /// an application that creates components from Rust types only (`mount`,
    /// declarative views, `.vue`): the components it never creates are not
    /// linked. Tag-based binding (`bind_semantic`) then refuses built-ins.
    const BUILTINS: nana_ui_runtime::BuiltinComponents = nana_ui_runtime::BuiltinComponents::Full;
    /// The `UiReady` point of startup (see [`crate::startup`]): the host can
    /// draw an ordinary document now. Create the state the first screen needs
    /// and start the rest as tasks; [`Self::build`] follows immediately.
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
    /// Observe framework window lifecycle events without importing native events.
    /// `Closed` is delivered after the document is removed and `window_closed` returns.
    /// Whether a `CloseRequested` closes the window is [`Self::close_requested`]'s answer.
    fn window_event(
        &mut self,
        _event: &WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }
    /// The system or the title bar asked to close `id`. The default answer
    /// closes it. Answer without [`WindowCommand::Close`] to keep it open —
    /// to ask first, or to hide to the tray — and close it later from
    /// [`Self::update`] once that is decided.
    ///
    /// [`WindowCommand::Close`]: nana_ui_platform::host::WindowCommand::Close
    fn close_requested(
        &mut self,
        id: WindowId,
        _windows: &mut HashMap<WindowId, ApplicationWindow>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate {
            window_commands: vec![nana_ui_platform::host::WindowCommand::Close(id)],
            ..RuntimeProgramUpdate::default()
        }
    }
    fn theme(&self) -> Arc<CompiledTheme> {
        builtin_theme_arc(ThemeAppearance::Dark)
    }
    /// What this process needs from its GPU backend. See
    /// [`RuntimeProgram::gpu_backend_policy`].
    fn gpu_backend_policy() -> crate::GpuBackendPolicy {
        crate::GpuBackendPolicy::Plain
    }
    /// Material the primary window is created with, before the first
    /// document exists. See [`RuntimeProgram::startup_window_material_mode`].
    fn startup_window_material_mode() -> crate::MaterialEffect {
        crate::MaterialEffect::Solid
    }
    /// Per-window material: a transparent overlay window answers
    /// `Transparent` for its id. See [`RuntimeProgram::window_material_mode_for`].
    fn window_material_mode_for(&self, _id: WindowId) -> crate::MaterialEffect {
        crate::MaterialEffect::Solid
    }
    /// Per-window backdrop opacity; foreground content keeps its own alpha.
    /// See [`RuntimeProgram::appearance_backdrop_opacity_for`].
    fn appearance_backdrop_opacity_for(&self, _id: WindowId) -> f32 {
        nana_ui_core::AppearanceSettings::DEFAULT_BACKDROP_OPACITY
    }
    /// Raw input after Runtime dispatch, with every window's document in
    /// reach. See [`RuntimeProgram::input_event`].
    fn input_event(
        &mut self,
        _id: WindowId,
        _input: RoutedInput<'_>,
        _windows: &mut HashMap<WindowId, ApplicationWindow>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, nana_ui_runtime::FrameworkError> {
        Ok(RuntimeProgramUpdate::default())
    }
    /// Application-owned wake deadline, independent of redraw cadence. See
    /// [`RuntimeProgram::next_wakeup`].
    fn next_wakeup(&self) -> Option<Instant> {
        None
    }
    /// The deadline [`Self::next_wakeup`] named has passed.
    fn wake(
        &mut self,
        _now: Instant,
        _windows: &mut HashMap<WindowId, ApplicationWindow>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
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
    /// A new frame of the window's [`ApplicationWindow::output`]. See
    /// [`RuntimeProgram::window_output_frame`].
    fn output_frame(
        &mut self,
        _window: &mut ApplicationWindow,
        _frame: &crate::WindowOutputFrame,
        _context: &RuntimeProgramContext<Self::Message>,
    ) {
    }
    /// The window's output changed state. See
    /// [`RuntimeProgram::window_output_status`].
    fn output_status(
        &mut self,
        _window: &mut ApplicationWindow,
        _status: crate::WindowOutputStatus,
        _context: &RuntimeProgramContext<Self::Message>,
    ) {
    }
    fn host_failure(&mut self, failure: crate::HostFailure) {
        eprintln!("NanaUI host failure: {failure:?}");
    }
    /// Whether the primary window's first document may replace the Early
    /// Splash as soon as it has a frame. See
    /// [`RuntimeProgram::startup_takeover`].
    fn startup_takeover(&self) -> crate::StartupTakeover {
        crate::StartupTakeover::Immediate
    }
    /// The startup moved to a later phase. See
    /// [`RuntimeProgram::startup_changed`].
    fn startup_changed(
        &mut self,
        _status: &crate::StartupStatus,
        _windows: &mut HashMap<WindowId, ApplicationWindow>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }
}

pub struct RuntimeApplication<State: ApplicationState> {
    pub state: State,
    pub windows: HashMap<WindowId, ApplicationWindow>,
}

impl<State: ApplicationState> RuntimeProgram for RuntimeApplication<State> {
    type Message = State::Message;
    type Error = State::Error;
    fn gpu_backend_policy() -> crate::GpuBackendPolicy {
        State::gpu_backend_policy()
    }
    fn startup_window_material_mode() -> crate::MaterialEffect {
        State::startup_window_material_mode()
    }
    fn window_material_mode_for(&self, id: WindowId) -> crate::MaterialEffect {
        self.state.window_material_mode_for(id)
    }
    fn appearance_backdrop_opacity_for(&self, id: WindowId) -> f32 {
        self.state.appearance_backdrop_opacity_for(id)
    }
    fn input_event(
        &mut self,
        id: WindowId,
        input: RoutedInput<'_>,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, nana_ui_runtime::FrameworkError> {
        self.state
            .input_event(id, input, &mut self.windows, context)
    }
    fn next_wakeup(&self) -> Option<Instant> {
        self.state.next_wakeup()
    }
    fn wake(
        &mut self,
        now: Instant,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.state.wake(now, &mut self.windows, context)
    }
    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let mut state = State::initialize(context)?;
        let mut window = ApplicationWindow::for_state::<State>();
        state.build(&mut window, context)?;
        let _ = window
            .document
            .context_mut()
            .set_theme_tokens(state.theme());
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
    fn theme(&self) -> Arc<CompiledTheme> {
        self.state.theme()
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
    fn resource_fetch_host(&self, id: WindowId) -> Option<nana_ui_platform::SharedFetchHost> {
        self.windows
            .get(&id)
            .and_then(|window| window.fetch_host.clone())
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
    fn window_output(&self, id: WindowId) -> Option<crate::WindowOutputConfig> {
        self.windows.get(&id).and_then(|window| window.output)
    }
    fn window_output_frame(
        &mut self,
        id: WindowId,
        frame: &crate::WindowOutputFrame,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if let Some(window) = self.windows.get_mut(&id) {
            self.state.output_frame(window, frame, context);
        }
    }
    fn window_output_status(
        &mut self,
        id: WindowId,
        status: crate::WindowOutputStatus,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if let Some(window) = self.windows.get_mut(&id) {
            self.state.output_status(window, status, context);
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
    fn initialize_window(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), String> {
        let mut window = ApplicationWindow::for_state::<State>();
        self.state
            .build(&mut window, context)
            .map_err(|error| error.to_string())?;
        let _ = window
            .document
            .context_mut()
            .set_theme_tokens(self.state.theme());
        self.windows.insert(id, window);
        Ok(())
    }

    fn discard_window(&mut self, id: WindowId) {
        self.windows.remove(&id);
        self.state.window_closed(id);
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        if let WindowEvent::Closed { id } = &event {
            self.windows.remove(id);
            self.state.window_closed(*id);
        }
        let observed = self.state.window_event(&event, context);
        let answer = match event {
            WindowEvent::CloseRequested { id } => {
                self.state.close_requested(id, &mut self.windows, context)
            }
            _ => RuntimeProgramUpdate::default(),
        };
        answer.merge(observed)
    }
    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        self.state.rebuild_gpu(&mut self.windows, context);
    }
    fn host_failure(&mut self, failure: crate::HostFailure) {
        self.state.host_failure(failure);
    }
    fn startup_takeover(&self) -> crate::StartupTakeover {
        self.state.startup_takeover()
    }
    fn startup_changed(
        &mut self,
        status: &crate::StartupStatus,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.state
            .startup_changed(status, &mut self.windows, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Overlay {
        overlay: WindowId,
        deadline: Instant,
    }

    impl ApplicationState for Overlay {
        type Message = ();
        type Error = String;
        fn initialize(_: &RuntimeProgramContext<()>) -> Result<Self, String> {
            unreachable!("constructed directly")
        }
        fn build(
            &mut self,
            _: &mut ApplicationWindow,
            _: &RuntimeProgramContext<()>,
        ) -> Result<(), String> {
            Ok(())
        }
        fn gpu_backend_policy() -> crate::GpuBackendPolicy {
            crate::GpuBackendPolicy::CompositionCapable
        }
        fn window_material_mode_for(&self, id: WindowId) -> crate::MaterialEffect {
            if id == self.overlay {
                crate::MaterialEffect::Transparent
            } else {
                crate::MaterialEffect::Solid
            }
        }
        fn appearance_backdrop_opacity_for(&self, id: WindowId) -> f32 {
            if id == self.overlay { 0.0 } else { 1.0 }
        }
        fn next_wakeup(&self) -> Option<Instant> {
            Some(self.deadline)
        }
    }

    #[test]
    fn an_application_state_answers_the_window_hooks_without_a_program_shim() {
        let overlay = WindowId(7);
        let other = WindowId(8);
        let deadline = Instant::now();
        let program = RuntimeApplication {
            state: Overlay { overlay, deadline },
            windows: HashMap::new(),
        };
        assert_eq!(
            <RuntimeApplication<Overlay> as RuntimeProgram>::gpu_backend_policy(),
            crate::GpuBackendPolicy::CompositionCapable
        );
        assert_eq!(
            program.window_material_mode_for(overlay),
            crate::MaterialEffect::Transparent
        );
        assert_eq!(
            program.window_material_mode_for(other),
            crate::MaterialEffect::Solid
        );
        assert_eq!(program.appearance_backdrop_opacity_for(overlay), 0.0);
        assert_eq!(RuntimeProgram::next_wakeup(&program), Some(deadline));
    }

    fn program_context(id: WindowId) -> RuntimeProgramContext<()> {
        RuntimeProgramContext::new(
            id,
            nana_ui_platform::WindowGeometry::default(),
            crate::test_gpu::context(),
            crate::ResolvedWindowPresentation::closed(),
            crate::CompositionWork::default(),
            Arc::new(|_| {}),
            std::sync::mpsc::sync_channel(1).0,
            None,
            crate::startup::StartupHandle::detached(),
        )
    }

    /// Asks before closing: keeps the window and remembers the request.
    #[derive(Default)]
    struct Confirming {
        asked: Vec<WindowId>,
        observed: usize,
    }

    impl ApplicationState for Confirming {
        type Message = ();
        type Error = String;
        fn initialize(_: &RuntimeProgramContext<()>) -> Result<Self, String> {
            unreachable!("constructed directly")
        }
        fn build(
            &mut self,
            _: &mut ApplicationWindow,
            _: &RuntimeProgramContext<()>,
        ) -> Result<(), String> {
            Ok(())
        }
        fn window_event(
            &mut self,
            event: &WindowEvent,
            _: &RuntimeProgramContext<()>,
        ) -> RuntimeProgramUpdate {
            if matches!(event, WindowEvent::CloseRequested { .. }) {
                self.observed += 1;
            }
            RuntimeProgramUpdate::default()
        }
        fn close_requested(
            &mut self,
            id: WindowId,
            _: &mut HashMap<WindowId, ApplicationWindow>,
            _: &RuntimeProgramContext<()>,
        ) -> RuntimeProgramUpdate {
            self.asked.push(id);
            RuntimeProgramUpdate::redraw(id)
        }
    }

    /// A close request closes the window by default; a state that answers
    /// without `Close` keeps it open, and `window_event` still observes it.
    #[test]
    fn a_close_request_closes_unless_the_state_answers_otherwise() {
        use nana_ui_platform::host::WindowCommand;

        let id = WindowId(3);
        let context = program_context(id);
        let mut closing = RuntimeApplication {
            state: Overlay {
                overlay: id,
                deadline: Instant::now(),
            },
            windows: HashMap::new(),
        };
        let answer = RuntimeProgram::window_event(
            &mut closing,
            WindowEvent::CloseRequested { id },
            &context,
        );
        assert_eq!(answer.window_commands, vec![WindowCommand::Close(id)]);

        let mut confirming = RuntimeApplication {
            state: Confirming::default(),
            windows: HashMap::new(),
        };
        let answer = RuntimeProgram::window_event(
            &mut confirming,
            WindowEvent::CloseRequested { id },
            &context,
        );
        assert!(answer.window_commands.is_empty(), "{answer:?}");
        assert!(!answer.exit);
        assert_eq!(answer.redraw, crate::RuntimeRedraw::Window(id));
        assert_eq!(confirming.state.asked, [id]);
        assert_eq!(confirming.state.observed, 1);
    }
}
