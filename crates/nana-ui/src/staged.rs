//! A program built in stages under the startup splash.
//!
//! Some programs cannot exist when the host starts: they load settings on a
//! worker, open a device, mount their pages one per turn so the splash keeps
//! presenting. [`Staged`] is the [`RuntimeProgram`] the host runs for them. It
//! answers for a [`Launch`] (a placeholder document and launch messages) until
//! the launch hands over the real program, then forwards every hook to that
//! program unchanged, so a hook added to [`RuntimeProgram`] reaches it without
//! the application forwarding anything. Once the program exists, `Staged`
//! takes the splash over for it when the program's own
//! [`RuntimeProgram::startup_takeover`] is [`StartupTakeover::Immediate`],
//! retrying while the host has no ticket yet.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_ui_core::{CompiledTheme, SemanticColor, builtin_theme_arc};
use nana_ui_platform::{WindowEvent, WindowId};
use nana_ui_runtime::{
    AccessibilityActionRequest, AccessibilityUpdate, AnimationFrame, FrameworkError,
};
use nana_ui_scene::{DocumentAccessError, RuntimeDocument};

use crate::{
    FrameDemand, HostFailure, HostTextureRegistry, MaterialEffect, RoutedInput, RuntimeProgram,
    RuntimeProgramContext, RuntimeProgramUpdate, SceneGpuRendererRegistry, StartupTakeover,
    ThemeAppearance,
};

/// The message type a launch shares with the program it builds.
pub type LaunchMessage<L> = <<L as Launch>::Program as RuntimeProgram>::Message;
/// The error type a launch shares with the program it builds.
pub type LaunchError<L> = <<L as Launch>::Program as RuntimeProgram>::Error;

/// What a launch message did.
pub enum LaunchUpdate<P> {
    /// Still launching; apply `update`.
    Continue(RuntimeProgramUpdate),
    /// The program is built: [`Staged`] forwards to it from now on, after
    /// applying `update` (commands the program queued while it was built,
    /// a redraw of its first surface).
    Ready(P, RuntimeProgramUpdate),
}

/// The part of a [`Staged`] program that runs before the program exists.
pub trait Launch: Sized + 'static {
    /// What the launch builds.
    type Program: RuntimeProgram;

    /// How long, from [`Launch::initialize`], the splash may wait for the
    /// built program to take it over before [`Launch::takeover_failed`].
    const TAKEOVER_TIMEOUT: Duration = Duration::from_secs(30);

    fn initialize(
        context: &RuntimeProgramContext<LaunchMessage<Self>>,
    ) -> Result<(Self, Vec<LaunchMessage<Self>>), LaunchError<Self>>;

    /// A message while launching. Messages meant for the program that arrive
    /// before it exists are the launch's to keep or drop.
    fn update(
        &mut self,
        message: LaunchMessage<Self>,
        context: &RuntimeProgramContext<LaunchMessage<Self>>,
    ) -> LaunchUpdate<Self::Program>;

    /// The primary window's document until the program exists; no other
    /// window opens before then.
    fn document(&self) -> &RuntimeDocument;

    fn document_mut(&mut self) -> &mut RuntimeDocument;

    /// The appearance the splash and the hidden primary window use before the
    /// program exists.
    fn theme_mode(&self) -> ThemeAppearance {
        ThemeAppearance::Dark
    }

    fn window_material_mode(&self) -> MaterialEffect {
        Self::Program::startup_window_material_mode()
    }

    fn window_background(&self) -> Option<SemanticColor> {
        None
    }

    /// The splash could not be taken over in time: what to do instead.
    /// Exits by default.
    fn takeover_failed(error: &str) -> RuntimeProgramUpdate {
        let _ = error;
        RuntimeProgramUpdate::exit()
    }
}

/// How often a takeover the host had no ticket for, or refused, is asked again.
const TAKEOVER_RETRY: Duration = Duration::from_millis(50);

enum Stage<L: Launch> {
    Launching(L),
    Running(L::Program),
}

/// The [`RuntimeProgram`] of a program built by a [`Launch`]. Run it with
/// [`crate::run_runtime`] as `Staged<MyLaunch>`.
pub struct Staged<L: Launch> {
    stage: Stage<L>,
    /// The clock the host gave last, for the program once it exists.
    animation_epoch: Option<Instant>,
    takeover: Takeover,
}

struct Takeover {
    /// Whether the splash is the program's, or is being handed to it.
    settled: bool,
    retry_at: Option<Instant>,
    deadline: Instant,
}

impl<L: Launch> Staged<L> {
    /// The built program, once the launch has handed it over.
    pub fn program(&self) -> Option<&L::Program> {
        match &self.stage {
            Stage::Running(program) => Some(program),
            Stage::Launching(_) => None,
        }
    }

    pub fn program_mut(&mut self) -> Option<&mut L::Program> {
        match &mut self.stage {
            Stage::Running(program) => Some(program),
            Stage::Launching(_) => None,
        }
    }

    /// The launch, while the program is still being built.
    pub fn launch(&self) -> Option<&L> {
        match &self.stage {
            Stage::Launching(launch) => Some(launch),
            Stage::Running(_) => None,
        }
    }

    fn ready(
        &mut self,
        mut program: L::Program,
        update: RuntimeProgramUpdate,
        context: &RuntimeProgramContext<LaunchMessage<L>>,
    ) -> RuntimeProgramUpdate {
        if let Some(epoch) = self.animation_epoch {
            program.sync_animation_clock(epoch);
        }
        let immediate = program.startup_takeover() == StartupTakeover::Immediate;
        self.stage = Stage::Running(program);
        if !immediate {
            // The program asks for the splash itself.
            self.takeover.settled = true;
            return update;
        }
        // The first real surface is built by now; asking earlier could bind
        // the splash to the placeholder frame.
        match self.take_over(context) {
            Some(failed) => failed,
            None => update,
        }
    }

    /// Ask the host to hand the splash over; `Some` once that has failed for
    /// good.
    fn take_over(
        &mut self,
        context: &RuntimeProgramContext<LaunchMessage<L>>,
    ) -> Option<RuntimeProgramUpdate> {
        if self.takeover.settled {
            return None;
        }
        let now = Instant::now();
        let status = context.startup().status();
        if status.phase == crate::StartupPhase::HandedOff {
            self.takeover.settled = true;
            self.takeover.retry_at = None;
            return None;
        }
        let error = match status.ticket {
            Some(ticket) => match context.startup().take_over(ticket) {
                Ok(()) | Err(crate::StartupError::AlreadyHandedOff) => {
                    self.takeover.settled = true;
                    self.takeover.retry_at = None;
                    return None;
                }
                Err(error) => error.to_string(),
            },
            None => "no ticket was available".to_owned(),
        };
        if now >= self.takeover.deadline {
            self.takeover.settled = true;
            self.takeover.retry_at = None;
            return Some(L::takeover_failed(&format!(
                "startup takeover timed out: {error}"
            )));
        }
        self.takeover.retry_at = Some(now + TAKEOVER_RETRY);
        None
    }
}

impl<L: Launch> RuntimeProgram for Staged<L> {
    type Message = LaunchMessage<L>;
    type Error = LaunchError<L>;

    fn gpu_backend_policy() -> crate::GpuBackendPolicy {
        L::Program::gpu_backend_policy()
    }

    fn startup_window_material_mode() -> MaterialEffect {
        L::Program::startup_window_material_mode()
    }

    #[cfg(target_os = "windows")]
    fn native_content_frame(
        &mut self,
        id: WindowId,
        composition: &crate::WindowsCompositionTree,
        regions: &[crate::NativeContentRegion],
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), String> {
        match &mut self.stage {
            Stage::Running(program) => {
                program.native_content_frame(id, composition, regions, context)
            }
            Stage::Launching(_) if regions.is_empty() => Ok(()),
            Stage::Launching(_) => Err("the program is still launching".into()),
        }
    }

    fn native_browser_requests(&self, id: WindowId) -> Vec<crate::NativeBrowserRequest> {
        self.program()
            .map(|program| program.native_browser_requests(id))
            .unwrap_or_default()
    }

    fn native_browser_event(
        &mut self,
        id: WindowId,
        event: crate::NativeBrowserEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.program_mut()
            .map(|program| program.native_browser_event(id, event, context))
            .unwrap_or_default()
    }

    fn web_surface_requests(&self) -> Vec<crate::WebSurfaceRequest> {
        self.program()
            .map(RuntimeProgram::web_surface_requests)
            .unwrap_or_default()
    }

    fn web_surface_event(
        &mut self,
        event: crate::WebSurfaceNotice,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.program_mut()
            .map(|program| program.web_surface_event(event, context))
            .unwrap_or_default()
    }

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let (launch, messages) = L::initialize(context)?;
        Ok((
            Self {
                stage: Stage::Launching(launch),
                animation_epoch: None,
                takeover: Takeover {
                    settled: false,
                    retry_at: None,
                    deadline: Instant::now() + L::TAKEOVER_TIMEOUT,
                },
            },
            messages,
        ))
    }

    /// Read once, right after `initialize`, when there is no program yet:
    /// the splash stays until `Staged` hands it to the program.
    fn startup_takeover(&self) -> StartupTakeover {
        StartupTakeover::Deferred
    }

    fn startup_changed(
        &mut self,
        status: &crate::StartupStatus,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.program_mut()
            .map(|program| program.startup_changed(status, context))
            .unwrap_or_default()
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        match &self.stage {
            Stage::Running(program) => program.with_document(id, f),
            Stage::Launching(launch) if id == WindowId::PRIMARY => Ok(Some(f(launch.document()))),
            Stage::Launching(_) => Ok(None),
        }
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        match &mut self.stage {
            Stage::Running(program) => program.with_document_mut(id, f),
            Stage::Launching(launch) if id == WindowId::PRIMARY => {
                Ok(Some(f(launch.document_mut())))
            }
            Stage::Launching(_) => Ok(None),
        }
    }

    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        let launched = match &mut self.stage {
            Stage::Running(program) => return program.update(message, context),
            Stage::Launching(launch) => launch.update(message, context),
        };
        match launched {
            LaunchUpdate::Continue(update) => update,
            LaunchUpdate::Ready(program, update) => self.ready(program, update, context),
        }
    }

    fn theme(&self) -> Arc<CompiledTheme> {
        match &self.stage {
            Stage::Running(program) => program.theme(),
            Stage::Launching(launch) => builtin_theme_arc(launch.theme_mode()),
        }
    }

    fn theme_mode(&self) -> ThemeAppearance {
        match &self.stage {
            Stage::Running(program) => program.theme_mode(),
            Stage::Launching(launch) => launch.theme_mode(),
        }
    }

    fn window_material_mode(&self) -> MaterialEffect {
        match &self.stage {
            Stage::Running(program) => program.window_material_mode(),
            Stage::Launching(launch) => launch.window_material_mode(),
        }
    }

    fn window_background(&self) -> Option<SemanticColor> {
        match &self.stage {
            Stage::Running(program) => program.window_background(),
            Stage::Launching(launch) => launch.window_background(),
        }
    }

    fn appearance_backdrop_opacity(&self) -> f32 {
        match &self.stage {
            Stage::Running(program) => program.appearance_backdrop_opacity(),
            Stage::Launching(_) => 1.0,
        }
    }

    fn window_material_mode_for(&self, id: WindowId) -> MaterialEffect {
        match &self.stage {
            Stage::Running(program) => program.window_material_mode_for(id),
            Stage::Launching(launch) => launch.window_material_mode(),
        }
    }

    fn appearance_backdrop_opacity_for(&self, id: WindowId) -> f32 {
        match &self.stage {
            Stage::Running(program) => program.appearance_backdrop_opacity_for(id),
            Stage::Launching(_) => 1.0,
        }
    }

    fn host_textures(&self, id: WindowId) -> Option<HostTextureRegistry> {
        self.program()?.host_textures(id)
    }

    fn scene_gpu_renderers(&self, id: WindowId) -> Option<SceneGpuRendererRegistry> {
        self.program()?.scene_gpu_renderers(id)
    }

    fn scene_resource_producers(
        &self,
        id: WindowId,
    ) -> Option<crate::SceneResourceProducerRegistry> {
        self.program()?.scene_resource_producers(id)
    }

    fn resource_fetch_host(&self, id: WindowId) -> Option<nana_ui_platform::SharedFetchHost> {
        self.program()?.resource_fetch_host(id)
    }

    fn prepare_window_frame(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if let Some(program) = self.program_mut() {
            program.prepare_window_frame(id, context);
        }
    }

    fn window_frame_presented(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.program_mut()
            .map(|program| program.window_frame_presented(id, context))
            .unwrap_or_default()
    }

    fn output_trace_enabled(&self, id: WindowId) -> bool {
        self.program()
            .is_some_and(|program| program.output_trace_enabled(id))
    }

    fn window_output_trace(
        &mut self,
        id: WindowId,
        trace: crate::output::OutputTraceSnapshot,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if let Some(program) = self.program_mut() {
            program.window_output_trace(id, trace, context);
        }
    }

    fn window_output(&self, id: WindowId) -> Option<crate::WindowOutputConfig> {
        self.program()?.window_output(id)
    }

    fn window_output_frame(
        &mut self,
        id: WindowId,
        frame: &crate::WindowOutputFrame,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if let Some(program) = self.program_mut() {
            program.window_output_frame(id, frame, context);
        }
    }

    fn window_output_status(
        &mut self,
        id: WindowId,
        status: crate::WindowOutputStatus,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if let Some(program) = self.program_mut() {
            program.window_output_status(id, status, context);
        }
    }

    fn bind_window(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.program_mut()
            .map(|program| program.bind_window(id, context))
            .unwrap_or_default()
    }

    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        if let Some(program) = self.program_mut() {
            program.rebuild_gpu(context);
        }
    }

    fn host_failure(&mut self, failure: HostFailure) {
        if let Some(program) = self.program_mut() {
            program.host_failure(failure);
        }
    }

    fn input_event(
        &mut self,
        id: WindowId,
        input: RoutedInput<'_>,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        match self.program_mut() {
            Some(program) => program.input_event(id, input, context),
            None => Ok(RuntimeProgramUpdate::default()),
        }
    }

    fn initialize_window(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), String> {
        match &mut self.stage {
            Stage::Running(program) => program.initialize_window(id, context),
            Stage::Launching(_) if id == WindowId::PRIMARY => Ok(()),
            Stage::Launching(_) => Err("no window opens while the program is launching".into()),
        }
    }

    fn discard_window(&mut self, id: WindowId) {
        if let Some(program) = self.program_mut() {
            program.discard_window(id);
        }
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.program_mut()
            .map(|program| program.window_event(event, context))
            .unwrap_or_default()
    }

    fn frame_demand(&self, id: WindowId) -> FrameDemand {
        self.program()
            .map_or(FrameDemand::OnDemand, |program| program.frame_demand(id))
    }

    fn max_frame_rate(&self, id: WindowId) -> Option<std::num::NonZeroU32> {
        self.program()
            .and_then(|program| program.max_frame_rate(id))
    }

    fn next_wakeup(&self) -> Option<Instant> {
        let program = self.program().and_then(RuntimeProgram::next_wakeup);
        match (program, self.takeover.retry_at) {
            (Some(program), Some(retry)) => Some(program.min(retry)),
            (program, retry) => program.or(retry),
        }
    }

    fn wake(
        &mut self,
        now: Instant,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        if self.program().is_some() && self.takeover.retry_at.is_some_and(|retry| now >= retry) {
            self.takeover.retry_at = None;
            if let Some(failed) = self.take_over(context) {
                return failed;
            }
        }
        self.program_mut()
            .map(|program| program.wake(now, context))
            .unwrap_or_default()
    }

    fn sync_animation_clock(&mut self, epoch: Instant) {
        self.animation_epoch = Some(epoch);
        if let Some(program) = self.program_mut() {
            program.sync_animation_clock(epoch);
        }
    }

    fn animation_frame(
        &mut self,
        id: WindowId,
        frame: AnimationFrame,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        match self.program_mut() {
            Some(program) => program.animation_frame(id, frame, context),
            None => Ok(RuntimeProgramUpdate::default()),
        }
    }

    fn take_accessibility_update(&mut self, id: WindowId) -> Option<AccessibilityUpdate> {
        self.program_mut()?.take_accessibility_update(id)
    }

    fn accessibility_action(
        &mut self,
        id: WindowId,
        request: AccessibilityActionRequest,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        match self.program_mut() {
            Some(program) => program.accessibility_action(id, request, context),
            None => Ok(RuntimeProgramUpdate::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_runtime::DocumentId;

    const SCENE: SemanticColor = SemanticColor::rgb8(20, 80, 160);
    const PLACEHOLDER: SemanticColor = SemanticColor::rgb8(0, 0, 0);
    const EVERY_FRAME: FrameDemand = FrameDemand::Continuous(std::num::NonZeroU32::MIN);

    fn document(id: u64) -> RuntimeDocument {
        RuntimeDocument::new(DocumentId::new(id).unwrap())
    }

    /// The program once built: answers with values of its own.
    struct Built {
        document: RuntimeDocument,
        clock: Option<Instant>,
        updates: usize,
    }

    impl RuntimeProgram for Built {
        type Message = u8;
        type Error = String;

        fn initialize(_context: &RuntimeProgramContext<u8>) -> Result<(Self, Vec<u8>), String> {
            unreachable!("a launch builds it")
        }

        fn with_document<R>(
            &self,
            id: WindowId,
            f: impl FnOnce(&RuntimeDocument) -> R,
        ) -> Result<Option<R>, DocumentAccessError> {
            Ok((id == WindowId::PRIMARY).then(|| f(&self.document)))
        }

        fn with_document_mut<R>(
            &mut self,
            id: WindowId,
            f: impl FnOnce(&mut RuntimeDocument) -> R,
        ) -> Result<Option<R>, DocumentAccessError> {
            Ok((id == WindowId::PRIMARY).then(|| f(&mut self.document)))
        }

        fn update(&mut self, _: u8, _: &RuntimeProgramContext<u8>) -> RuntimeProgramUpdate {
            self.updates += 1;
            RuntimeProgramUpdate::redraw(WindowId::PRIMARY)
        }

        fn theme_mode(&self) -> ThemeAppearance {
            ThemeAppearance::Light
        }

        fn window_background(&self) -> Option<SemanticColor> {
            Some(SCENE)
        }

        fn frame_demand(&self, _id: WindowId) -> FrameDemand {
            EVERY_FRAME
        }

        fn sync_animation_clock(&mut self, epoch: Instant) {
            self.clock = Some(epoch);
        }
    }

    /// Builds [`Built`] on its second message.
    struct Booting {
        placeholder: RuntimeDocument,
        steps: u8,
    }

    impl Launch for Booting {
        type Program = Built;
        const TAKEOVER_TIMEOUT: Duration = Duration::from_secs(60);

        fn initialize(_context: &RuntimeProgramContext<u8>) -> Result<(Self, Vec<u8>), String> {
            Ok((
                Self {
                    placeholder: document(31),
                    steps: 0,
                },
                Vec::new(),
            ))
        }

        fn update(&mut self, _: u8, _: &RuntimeProgramContext<u8>) -> LaunchUpdate<Built> {
            self.steps += 1;
            if self.steps < 2 {
                return LaunchUpdate::Continue(RuntimeProgramUpdate::default());
            }
            LaunchUpdate::Ready(
                Built {
                    document: document(1),
                    clock: None,
                    updates: 0,
                },
                RuntimeProgramUpdate::redraw_all(),
            )
        }

        fn document(&self) -> &RuntimeDocument {
            &self.placeholder
        }

        fn document_mut(&mut self) -> &mut RuntimeDocument {
            &mut self.placeholder
        }

        fn theme_mode(&self) -> ThemeAppearance {
            ThemeAppearance::Dark
        }

        fn window_background(&self) -> Option<SemanticColor> {
            Some(PLACEHOLDER)
        }
    }

    fn context() -> RuntimeProgramContext<u8> {
        RuntimeProgramContext::new(
            WindowId::PRIMARY,
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

    fn document_id(program: &Staged<Booting>) -> Option<u64> {
        program
            .with_document(WindowId::PRIMARY, |document| document.document().get())
            .unwrap()
    }

    /// Before the program exists the launch answers, with its placeholder for
    /// the primary window only; once it is built every hook is the program's
    /// and the splash is asked for on its behalf.
    #[test]
    fn a_staged_program_answers_for_its_launch_then_forwards_to_the_program() {
        let context = context();
        let (mut staged, _) = Staged::<Booting>::initialize(&context).unwrap();
        let epoch = Instant::now();
        staged.sync_animation_clock(epoch);

        assert_eq!(staged.startup_takeover(), StartupTakeover::Deferred);
        assert_eq!(document_id(&staged), Some(31));
        assert!(staged.with_document(WindowId(9), |_| ()).unwrap().is_none());
        assert_eq!(staged.window_background(), Some(PLACEHOLDER));
        assert_eq!(staged.theme_mode(), ThemeAppearance::Dark);
        assert_eq!(
            staged.frame_demand(WindowId::PRIMARY),
            FrameDemand::OnDemand
        );
        assert!(staged.initialize_window(WindowId(9), &context).is_err());

        assert_eq!(staged.update(0, &context), RuntimeProgramUpdate::default());
        assert!(staged.program().is_none());
        assert_eq!(
            staged.update(0, &context),
            RuntimeProgramUpdate::redraw_all()
        );

        let program = staged.program().expect("built on the second step");
        assert_eq!(program.clock, Some(epoch), "the clock reaches the program");
        assert_eq!(document_id(&staged), Some(1));
        assert_eq!(staged.window_background(), Some(SCENE));
        assert_eq!(staged.theme_mode(), ThemeAppearance::Light);
        assert_eq!(staged.frame_demand(WindowId::PRIMARY), EVERY_FRAME);
        staged.update(0, &context);
        assert_eq!(staged.program().unwrap().updates, 1, "messages are its now");

        // A detached host has no ticket: the takeover is asked again soon.
        let retry = staged.next_wakeup().expect("a takeover retry");
        assert!(retry > Instant::now() - Duration::from_secs(1));
    }
}
