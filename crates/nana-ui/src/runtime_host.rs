//! Backend-neutral application contract for the Nana Scene host.
//!
//! Applications own [`RuntimeDocument`] values and drive [`run_runtime`].
//! [`run_runtime`] is the product host entry and delegates to
//! [`crate::run_runtime_scene`].

use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::Instant;

use nana_ui_core::{SharedStore, memory_store};
use nana_ui_platform::host::WindowCommand;
use nana_ui_platform::{InputEvent, SystemAppearance, WindowEvent, WindowGeometry, WindowId};
use nana_ui_runtime::{
    AccessibilityActionRequest, AccessibilityUpdate, AnimationFrame, FrameworkError, StableNodeId,
    Task,
};
use nana_ui_scene::{DocumentAccessError, RuntimeDocument};

use crate::{
    HostTextureRegistry, HostedGpuResources, MaterialOutcome, SceneGpuRendererRegistry, ThemeMode,
};

pub use nana_ui_platform::WindowDescriptor;

/// Skip the raw program input hook when Runtime already consumed the event.
/// The Scene host does not use this gate; it always delivers `input_event`.
#[cfg(test)]
pub(crate) fn gated_runtime_input_update(
    disposition: nana_ui_platform::InputDisposition,
    id: WindowId,
    raw_input: impl FnOnce() -> Result<RuntimeProgramUpdate, FrameworkError>,
) -> RuntimeProgramUpdate {
    if disposition.prevent_default {
        RuntimeProgramUpdate::redraw(id)
    } else {
        raw_input().unwrap_or_else(|error| panic!("RuntimeProgram input handler failed: {error}"))
    }
}

pub(crate) fn gated_runtime_window_update(
    prevent_raw: bool,
    raw_event: impl FnOnce() -> RuntimeProgramUpdate,
) -> RuntimeProgramUpdate {
    if prevent_raw {
        RuntimeProgramUpdate::default()
    } else {
        raw_event()
    }
}

/// Host services that are safe to retain or invoke from application code.
/// Native window identities intentionally do not cross this boundary.
pub struct RuntimeProgramContext<Message: Send + 'static> {
    window_id: WindowId,
    /// Bound to the window generation live when the context was built, so a
    /// retained context never controls a later window with the same identity.
    window: Option<crate::WindowHandle>,
    window_tag: Option<Arc<str>>,
    geometry: WindowGeometry,
    gpu: HostedGpuResources,
    /// What this window presents, the target it reaches the screen through, and
    /// why either of them differs from what was asked for. One value, so a
    /// program cannot read a material and an alpha mode that disagree.
    presentation: crate::ResolvedWindowPresentation,
    composition_work: crate::CompositionWork,
    dispatch: Arc<dyn Fn(Message) + Send + Sync>,
    tasks: SyncSender<Task<Message>>,
    system_appearance: Option<SystemAppearance>,
    reduced_motion: bool,
    store: SharedStore,
}

// Cloning host handles never clones a message. A derived implementation would
// unnecessarily require Message: Clone, preventing move-only application input.
impl<Message: Send + 'static> Clone for RuntimeProgramContext<Message> {
    fn clone(&self) -> Self {
        Self {
            window_id: self.window_id,
            window: self.window.clone(),
            window_tag: self.window_tag.clone(),
            geometry: self.geometry,
            gpu: self.gpu.clone(),
            presentation: self.presentation,
            composition_work: self.composition_work,
            dispatch: Arc::clone(&self.dispatch),
            tasks: self.tasks.clone(),
            system_appearance: self.system_appearance,
            reduced_motion: self.reduced_motion,
            store: Arc::clone(&self.store),
        }
    }
}

impl<Message: Send + 'static> RuntimeProgramContext<Message> {
    #[expect(
        clippy::too_many_arguments,
        reason = "Host-owned resources form one callback context"
    )]
    pub(crate) fn new(
        window_id: WindowId,
        geometry: WindowGeometry,
        gpu: HostedGpuResources,
        presentation: crate::ResolvedWindowPresentation,
        composition_work: crate::CompositionWork,
        dispatch: Arc<dyn Fn(Message) + Send + Sync>,
        tasks: SyncSender<Task<Message>>,
        system_appearance: Option<SystemAppearance>,
    ) -> Self {
        Self {
            window_id,
            window: None,
            window_tag: None,
            geometry,
            gpu,
            presentation,
            composition_work,
            dispatch,
            tasks,
            system_appearance,
            reduced_motion: false,
            store: memory_store(),
        }
    }

    pub(crate) fn with_reduced_motion(mut self, reduced: bool) -> Self {
        self.reduced_motion = reduced;
        self
    }

    pub(crate) fn with_store(mut self, store: SharedStore) -> Self {
        self.store = store;
        self
    }

    pub(crate) fn with_windows(mut self, windows: &crate::WindowService) -> Self {
        self.window = Some(windows.handle(self.window_id));
        self
    }

    pub(crate) fn with_window_tag(mut self, tag: Option<Arc<str>>) -> Self {
        self.window_tag = tag;
        self
    }

    pub fn windows(&self) -> &crate::WindowService {
        self.window_handle().service()
    }

    pub fn window(&self) -> crate::WindowHandle {
        self.window_handle().clone()
    }

    fn window_handle(&self) -> &crate::WindowHandle {
        self.window
            .as_ref()
            .expect("window service is available in a native host")
    }

    pub const fn window_id(&self) -> WindowId {
        self.window_id
    }

    /// [`WindowDescriptor::tag`](crate::WindowDescriptor::tag) of this window,
    /// available from `initialize_window` / `ApplicationState::build` on.
    /// `None` once the window has closed.
    pub fn window_tag(&self) -> Option<&str> {
        self.window_tag.as_deref()
    }

    /// The system asks to reduce motion. `false` when the platform does not
    /// report it; changes arrive as `WindowEvent::ReducedMotionChanged`.
    pub const fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    pub const fn geometry(&self) -> WindowGeometry {
        self.geometry
    }

    pub fn gpu(&self) -> &HostedGpuResources {
        &self.gpu
    }

    /// What this window is actually presenting, with the fallback reason when
    /// the request could not be met.
    pub const fn material(&self) -> MaterialOutcome {
        self.presentation.effective()
    }

    /// What mirroring this window's scene into the platform compositor has
    /// cost so far. A settled window's counters stop moving, however many GPU
    /// frames it goes on presenting.
    pub const fn composition_work(&self) -> crate::CompositionWork {
        self.composition_work
    }

    /// The whole resolved presentation: requested and effective material, the
    /// surface alpha mode, the presentation target and any fallback from the
    /// target that was asked for.
    pub const fn presentation(&self) -> crate::ResolvedWindowPresentation {
        self.presentation
    }

    /// The operating system's light/dark preference when this context was
    /// built, or `None` on platforms that do not report one. Follow later
    /// changes through [`WindowEvent::AppearanceChanged`]; do not shell out to
    /// `defaults` or the registry.
    ///
    /// [`WindowEvent::AppearanceChanged`]: nana_ui_platform::WindowEvent::AppearanceChanged
    pub const fn system_appearance(&self) -> Option<SystemAppearance> {
        self.system_appearance
    }

    /// Process-level persistence. Default is memory-only; hosts inject a
    /// file-backed store through [`run_runtime_with_store`].
    pub fn store(&self) -> &SharedStore {
        &self.store
    }

    pub const fn surface_alpha_mode(&self) -> wgpu::CompositeAlphaMode {
        self.presentation.alpha_mode()
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RuntimeRedraw {
    #[default]
    None,
    Window(WindowId),
    Windows(Vec<WindowId>),
    All,
}

impl RuntimeRedraw {
    pub fn for_windows(windows: impl IntoIterator<Item = WindowId>) -> Self {
        let mut windows: Vec<_> = windows.into_iter().collect();
        windows.sort_by_key(|id| id.0);
        windows.dedup();
        match windows.as_slice() {
            [] => Self::None,
            [id] => Self::Window(*id),
            _ => Self::Windows(windows),
        }
    }
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

    pub(crate) fn merge(mut self, other: Self) -> Self {
        self.exit |= other.exit;
        self.window_commands.extend(other.window_commands);
        self.redraw = match (self.redraw, other.redraw) {
            (RuntimeRedraw::All, _) | (_, RuntimeRedraw::All) => RuntimeRedraw::All,
            (RuntimeRedraw::None, redraw) | (redraw, RuntimeRedraw::None) => redraw,
            (left, right) => {
                let into_windows = |redraw| match redraw {
                    RuntimeRedraw::Window(id) => vec![id],
                    RuntimeRedraw::Windows(ids) => ids,
                    _ => Vec::new(),
                };
                RuntimeRedraw::for_windows(
                    into_windows(left).into_iter().chain(into_windows(right)),
                )
            }
        };
        self
    }
}

/// A degraded outcome the Scene host recovered from by dropping the failed
/// callback's effect or skipping one frame. Reported through
/// [`RuntimeProgram::host_failure`] so programs decide how to surface the
/// failure instead of the host panicking inside the platform event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostFailure {
    DocumentAccess { window: WindowId, error: String },
    AccessibilityAction { window: WindowId, error: String },
    ImeDispatch { window: WindowId, error: String },
    AnimationFrame { window: WindowId, error: String },
    InputDispatch { window: WindowId, error: String },
    InputHandler { window: WindowId, error: String },
    MissingDocument { window: WindowId },
    FrameDidNotSettle { window: WindowId, error: String },
    ResourceProduction { window: WindowId, error: String },
    UnpaintableScene { window: WindowId, error: String },
    SurfaceRecovery { window: WindowId, error: String },
}

impl HostFailure {
    pub fn window(&self) -> WindowId {
        match self {
            Self::DocumentAccess { window, .. }
            | Self::AccessibilityAction { window, .. }
            | Self::ImeDispatch { window, .. }
            | Self::AnimationFrame { window, .. }
            | Self::InputDispatch { window, .. }
            | Self::InputHandler { window, .. }
            | Self::MissingDocument { window }
            | Self::FrameDidNotSettle { window, .. }
            | Self::ResourceProduction { window, .. }
            | Self::UnpaintableScene { window, .. }
            | Self::SurfaceRecovery { window, .. } => *window,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::DocumentAccess { error, .. }
            | Self::AccessibilityAction { error, .. }
            | Self::ImeDispatch { error, .. }
            | Self::AnimationFrame { error, .. }
            | Self::InputDispatch { error, .. }
            | Self::InputHandler { error, .. }
            | Self::FrameDidNotSettle { error, .. }
            | Self::ResourceProduction { error, .. }
            | Self::UnpaintableScene { error, .. }
            | Self::SurfaceRecovery { error, .. } => Some(error),
            Self::MissingDocument { .. } => None,
        }
    }
}

/// Whether a fault may be written now for the variant whose last write time
/// is in `slot` (milliseconds, 0 = never): at most one per second, and only
/// one of several racing threads wins.
fn claim_fault_slot(slot: &std::sync::atomic::AtomicU64, now_ms: u64) -> bool {
    use std::sync::atomic::Ordering;
    let last = slot.load(Ordering::Relaxed);
    if last != 0 && now_ms.saturating_sub(last) < 1000 {
        return false;
    }
    slot.compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
}

/// Record `failure` in diagnostics and hand it on.
pub(crate) fn recorded(failure: HostFailure) -> HostFailure {
    failure.record_diagnostics();
    failure
}

impl HostFailure {
    /// Stable numeric code of the variant, written to diagnostics logs.
    /// Append-only.
    pub fn code(&self) -> u64 {
        match self {
            Self::DocumentAccess { .. } => 1,
            Self::AccessibilityAction { .. } => 2,
            Self::ImeDispatch { .. } => 3,
            Self::AnimationFrame { .. } => 4,
            Self::InputDispatch { .. } => 5,
            Self::InputHandler { .. } => 6,
            Self::MissingDocument { .. } => 7,
            Self::FrameDidNotSettle { .. } => 8,
            Self::ResourceProduction { .. } => 9,
            Self::UnpaintableScene { .. } => 10,
            Self::SurfaceRecovery { .. } => 11,
        }
    }

    /// Record this failure in diagnostics (Issue #227). Hosts call it where
    /// they report the failure to the program; free when diagnostics are off.
    ///
    /// Every call is counted; the fault itself is written at most once per
    /// second per variant, because a failure that recurs every frame (a
    /// frame that never settles, a broken input handler) would otherwise
    /// flood the log and the fault ring.
    pub fn record_diagnostics(&self) {
        use nana_diagnostics::framework::host;
        use std::sync::atomic::AtomicU64;
        static LAST_FAULT_MS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
        static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

        if !nana_diagnostics::enabled(host::FAILURE.severity) {
            return;
        }
        nana_diagnostics::metric!(host::FAILURES);
        // +1 so a failure in the first millisecond is not read as "never".
        let now_ms = EPOCH
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
            + 1;
        if !claim_fault_slot(
            &LAST_FAULT_MS[self.code() as usize % LAST_FAULT_MS.len()],
            now_ms,
        ) {
            return;
        }
        match self.error() {
            Some(error) => nana_diagnostics::fault!(
                host::FAILURE,
                window = self.window().0,
                kind = self.code();
                "{self}: {error}"
            ),
            None => nana_diagnostics::fault!(
                host::FAILURE,
                window = self.window().0,
                kind = self.code();
                "{self}"
            ),
        }
    }
}

impl fmt::Display for HostFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "host failure on window {}", self.window().0)?;
        match self {
            Self::SurfaceRecovery { .. } => formatter.write_str(": surface recovery pending"),
            Self::DocumentAccess { .. } => formatter.write_str(": document access failed"),
            Self::AccessibilityAction { .. } => {
                formatter.write_str(": accessibility action failed")
            }
            Self::ImeDispatch { .. } => formatter.write_str(": IME dispatch failed"),
            Self::AnimationFrame { .. } => formatter.write_str(": animation handler failed"),
            Self::InputDispatch { .. } => formatter.write_str(": input dispatch failed"),
            Self::InputHandler { .. } => formatter.write_str(": input handler failed"),
            Self::MissingDocument { .. } => formatter.write_str(": no document for window"),
            Self::FrameDidNotSettle { .. } => formatter.write_str(": frame did not settle"),
            Self::ResourceProduction { .. } => formatter.write_str(": resource production failed"),
            Self::UnpaintableScene { .. } => formatter.write_str(": unpaintable UiScene"),
        }
    }
}

/// Canonical retained application contract for the Nana Scene host.
///
/// `Message` is for host-level work (windows, GPU, persistence). Control
/// interaction should update Runtime views through `on` / `observe`.
pub trait RuntimeProgram: Sized + 'static {
    type Message: Send + 'static;
    type Error: fmt::Display;

    /// What this process needs from its GPU backend.
    ///
    /// Process-wide, because the backend, adapter and device are: every window
    /// shares one of each. Asking for
    /// [`GpuBackendPolicy::CompositionCapable`] makes the compositor path
    /// *available*; it does not put any window on it. A window asks for the
    /// path with [`WindowDescriptor::surface`].
    ///
    /// [`GpuBackendPolicy::CompositionCapable`]: crate::GpuBackendPolicy::CompositionCapable
    /// [`WindowDescriptor::surface`]: crate::WindowDescriptor::surface
    fn gpu_backend_policy() -> crate::GpuBackendPolicy {
        crate::GpuBackendPolicy::Plain
    }

    /// Mirrors this frame's native-content regions into the window's
    /// DirectComposition tree.
    ///
    /// Called only when the regions differ from the ones this window was last
    /// given, so a program that maps them onto visuals one-to-one does no work
    /// on a frame whose native geometry did not move — however many GPU frames
    /// the UI presents in between.
    ///
    /// That also means this is *not* the only place a backend may touch its
    /// visuals. A backend with its own reason to change one — an engine frame
    /// arrived, the application hid a layer — mutates the
    /// [`WindowsNativeVisual`](crate::WindowsNativeVisual) it already holds,
    /// whenever it likes. Staging is what marks the tree dirty, so the next
    /// frame's commit publishes it; waiting for this callback would wait for a
    /// geometry change that may never come.
    ///
    /// The tree handed in stages changes; it cannot commit them. The Scene
    /// host publishes the transaction once per frame, so the backend and the
    /// host never submit the same tree.
    #[cfg(target_os = "windows")]
    fn native_content_frame(
        &mut self,
        _id: WindowId,
        _composition: &crate::WindowsCompositionTree,
        regions: &[crate::NativeContentRegion],
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), String> {
        if regions.is_empty() {
            Ok(())
        } else {
            Err("native content backend is unavailable".into())
        }
    }

    /// Native web content is anchored to BrowserView nodes; application chrome stays in Runtime.
    /// Omit a request to release its native child. Revisions must increase for every command.
    fn native_browser_requests(&self, _id: WindowId) -> Vec<crate::NativeBrowserRequest> {
        Vec::new()
    }

    fn native_browser_event(
        &mut self,
        _id: WindowId,
        _event: crate::NativeBrowserEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error>;

    /// Borrow one document for a synchronous operation. References cannot escape
    /// the callback; release the scope before invoking application or JS code.
    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError>;
    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError>;

    /// Apply a host-level message, on the frame after it was dispatched.
    ///
    /// `dispatch_program` coalesces to the latest message of each type;
    /// `dispatch_program_all` delivers every one in dispatch order. Keep this
    /// cheap; fill content in [`Self::bind_window`] after present.
    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate;

    fn theme_mode(&self) -> ThemeMode;

    fn window_material_mode(&self) -> crate::MaterialEffect {
        crate::MaterialEffect::Solid
    }

    /// The colour an opaque window surface clears to, and the colour a system
    /// material falls back to when the platform cannot honour it.
    ///
    /// `None` takes the installed theme's `background`, which is what a window
    /// whose content *is* the page wants. A host whose window is a frame around
    /// content of its own — a stage, a canvas, a video surface — answers with
    /// its own colour instead, and that colour does not move when the theme
    /// does. This is the window surface, which the host owns; it is not a
    /// semantic role, and nothing inside the document reads it.
    fn window_background(&self) -> Option<nana_ui_core::SemanticColor> {
        None
    }

    fn appearance_backdrop_opacity(&self) -> f32 {
        nana_ui_core::AppearanceSettings::DEFAULT_BACKDROP_OPACITY
    }

    /// Per-window material override. Existing applications retain their global policy.
    fn window_material_mode_for(&self, _id: WindowId) -> crate::MaterialEffect {
        self.window_material_mode()
    }

    /// Per-window backdrop opacity; does not change foreground content alpha.
    fn appearance_backdrop_opacity_for(&self, _id: WindowId) -> f32 {
        self.appearance_backdrop_opacity()
    }

    /// Product default: attach an existing sampleable texture to the tree.
    ///
    /// Pair with [`crate::GpuTextureView`] on the same slot, then update the
    /// view in [`Self::prepare_window_frame`].
    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        None
    }

    /// Advanced: explicitly register renderers for nodes drawn in the UI pass.
    /// No demo renderer is installed implicitly.
    fn scene_gpu_renderers(&self, _id: WindowId) -> Option<SceneGpuRendererRegistry> {
        None
    }

    /// Advanced: graph-scheduled offscreen on the HostTexture path.
    /// Same Device/Queue and host encoder; preparation precedes Scene sampling
    /// within the target's single submission.
    fn scene_resource_producers(
        &self,
        _id: WindowId,
    ) -> Option<crate::SceneResourceProducerRegistry> {
        None
    }

    /// Policy-gated egress for this window's `http(s)` `url(...)` images.
    ///
    /// Return the same host that governs the window's JS `fetch()`, so one
    /// policy covers both paths. `None` refuses every remote image.
    ///
    /// Called every frame. Return a clone of a host the application keeps, not
    /// one built per call: a new `Arc` is a new egress, so its images would be
    /// requested again and the window repainted on every frame.
    fn resource_fetch_host(&self, _id: WindowId) -> Option<nana_ui_platform::SharedFetchHost> {
        None
    }

    /// Acquire application-owned frame resources immediately before the host
    /// flushes and paints this window. Also runs when [`Self::frame_demand`] is
    /// due while the window is occluded, minimised, or has 0-size
    /// geometry (even if visible): that hidden tick does not flush, acquire a
    /// Surface, or call
    /// [`Self::window_frame_presented`]. Keep this method cheap; do not retire
    /// textures the last presented UI frame still samples.
    fn prepare_window_frame(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) {
    }

    /// Release resources retired by [`Self::prepare_window_frame`] only after
    /// the host has submitted and presented this window's frame. Hidden GPU
    /// ticks do not call this.
    fn window_frame_presented(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    /// Fill content after the host presented a frame that applied [`Self::update`].
    fn bind_window(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    fn rebuild_gpu(&mut self, _context: &RuntimeProgramContext<Self::Message>) {}

    /// Report a [`HostFailure`] the host has already recovered from: the
    /// failed callback's effect was dropped or the frame was skipped, and the
    /// event loop keeps running. Override to log, surface UI feedback, or
    /// exit; the default ignores the report.
    fn host_failure(&mut self, _failure: HostFailure) {}

    /// Receive raw input after Runtime dispatch. This is the only input hook;
    /// everything the host learned while dispatching the event travels in
    /// [`RoutedInput`], so a program never has to pick between overloads and
    /// silently lose the hit target or the disposition.
    fn input_event(
        &mut self,
        _id: WindowId,
        _input: RoutedInput<'_>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        Ok(RuntimeProgramUpdate::default())
    }

    /// Build the window document before the native window is published.
    /// Returning an error rolls the entire creation back; Ready is never sent.
    fn initialize_window(
        &mut self,
        id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(), String> {
        self.with_document(id, |_| ())
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "window document was not initialized".to_string())
    }

    /// Roll back application state after an unsuccessful window creation.
    fn discard_window(&mut self, _id: WindowId) {}

    /// The host never closes a window on its own. `CloseRequested`, from the
    /// system or the title-bar close button, closes the window only when the
    /// program answers with [`WindowCommand::Close`] or an exit; the default
    /// answers at once, and a program that must save first answers later.
    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::CloseRequested { id } => RuntimeProgramUpdate {
                window_commands: vec![WindowCommand::Close(id)],
                ..RuntimeProgramUpdate::default()
            },
            _ => RuntimeProgramUpdate::default(),
        }
    }

    /// Presentation cadence is independent of application task wakeups.
    fn frame_demand(&self, _id: WindowId) -> FrameDemand {
        FrameDemand::OnDemand
    }

    /// Application-owned wake deadline for sampled state, external runtimes,
    /// retry backoff or other work that must not depend on UI redraw cadence.
    fn next_wakeup(&self) -> Option<Instant> {
        None
    }

    fn wake(
        &mut self,
        _now: Instant,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    /// Align hosted program CSS animation sampling with the Scene host epoch.
    fn sync_animation_clock(&mut self, _epoch: Instant) {}

    fn animation_frame(
        &mut self,
        _id: WindowId,
        _frame: AnimationFrame,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        Ok(RuntimeProgramUpdate::default())
    }

    /// Drain accessibility work already applied to the retained world.
    /// Used when `RuntimeDocument::flush` is empty because a consumer flushed
    /// systems earlier in the same frame.
    fn take_accessibility_update(&mut self, _id: WindowId) -> Option<AccessibilityUpdate> {
        None
    }

    fn accessibility_action(
        &mut self,
        id: WindowId,
        request: AccessibilityActionRequest,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let changed = self
            .with_document_mut(id, |document| {
                let document_id = document.document();
                document
                    .context_mut()
                    .apply_accessibility_action(document_id, request)
            })
            .map_err(|error| {
                self.host_failure(recorded(HostFailure::DocumentAccess {
                    window: id,
                    error: error.to_string(),
                }));
                FrameworkError::InvalidInput
            })?
            .transpose()?
            .unwrap_or(false);
        Ok(if changed {
            RuntimeProgramUpdate::redraw(id)
        } else {
            RuntimeProgramUpdate::default()
        })
    }
}

/// Host boundary: report failed access after the document scope has ended.
pub(crate) trait HostDocumentAccess: RuntimeProgram {
    fn read_document<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Option<R> {
        match self.with_document(id, f) {
            Ok(value) => value,
            Err(error) => {
                self.host_failure(recorded(HostFailure::DocumentAccess {
                    window: id,
                    error: error.to_string(),
                }));
                None
            }
        }
    }
    fn write_document<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Option<R> {
        match self.with_document_mut(id, f) {
            Ok(value) => value,
            Err(error) => {
                self.host_failure(recorded(HostFailure::DocumentAccess {
                    window: id,
                    error: error.to_string(),
                }));
                None
            }
        }
    }
}
impl<T: RuntimeProgram> HostDocumentAccess for T {}

pub(crate) fn runtime_text_input_request(
    document: &RuntimeDocument,
) -> nana_ui_platform::TextInputRequest {
    if document
        .context()
        .terminal_accepts_input(document.document())
    {
        return nana_ui_platform::TextInputRequest {
            enabled: true,
            cursor_area: document
                .context()
                .terminal_caret_bounds(document.document())
                .map(|bounds| {
                    nana_ui_core::LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height)
                }),
            purpose: nana_ui_platform::TextInputPurpose::Normal,
        };
    }
    let focused = document
        .context()
        .focused_text_input(document.document())
        .map(|(target, _)| target)
        .filter(|target| {
            document
                .context()
                .world()
                .accessibility(*target)
                .is_some_and(|state| state.editable)
        });
    let cursor_area = focused
        .and_then(
            |target| match document.context().world().component_geometry(target) {
                Some(nana_ui_runtime::ComponentGeometry::TextInput {
                    caret: Some(caret), ..
                }) => Some(caret),
                _ => document.context().world().layout_box(target),
            },
        )
        .map(|layout| {
            nana_ui_core::LogicalRect::new(layout.x, layout.y, layout.width, layout.height)
        });
    let secure = focused
        .and_then(|target| document.context().world().standard_visual(target))
        .is_some_and(|visual| {
            matches!(
                visual,
                nana_ui_runtime::StandardVisual::TextInput { secure: true, .. }
            )
        });
    let purpose = if secure {
        nana_ui_platform::TextInputPurpose::Password
    } else {
        nana_ui_platform::TextInputPurpose::Normal
    };
    nana_ui_platform::TextInputRequest {
        enabled: focused.is_some(),
        cursor_area,
        purpose,
    }
}

/// Focused editor excerpt for winit surrounding text. Password / unfocused: `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImeSurroundingSnapshot {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}

const IME_SURROUNDING_MAX_BYTES: usize = 4000;

pub(crate) fn runtime_ime_surrounding(
    document: &RuntimeDocument,
) -> Option<ImeSurroundingSnapshot> {
    let request = runtime_text_input_request(document);
    if !request.enabled || request.purpose == nana_ui_platform::TextInputPurpose::Password {
        return None;
    }
    let (_, state) = document.context().focused_text_input(document.document())?;
    clip_ime_surrounding(&state.value, state.selection.focus, state.selection.anchor)
}

/// At most [`IME_SURROUNDING_MAX_BYTES`] of the value around the selection.
///
/// A selection that fits is reported whole, with the rest of the budget split
/// around it (unused space on one side goes to the other); only a selection
/// longer than the budget is cut, around its cursor. The window never splits a
/// character: [`nana_text::editable::ime::surrounding_window`].
fn clip_ime_surrounding(
    text: &str,
    cursor: usize,
    anchor: usize,
) -> Option<ImeSurroundingSnapshot> {
    if !text.is_char_boundary(cursor) || !text.is_char_boundary(anchor) {
        return None;
    }
    let selection = cursor.min(anchor)..cursor.max(anchor);
    let window = if selection.len() <= IME_SURROUNDING_MAX_BYTES {
        let spare = IME_SURROUNDING_MAX_BYTES - selection.len();
        let after_available = text.len() - selection.end;
        let before = selection
            .start
            .min((spare / 2).max(spare.saturating_sub(after_available)));
        nana_text::editable::ime::surrounding_window(text, selection, before, spare - before)
    } else {
        let half = IME_SURROUNDING_MAX_BYTES / 2;
        nana_text::editable::ime::surrounding_window(text, cursor..cursor, half, half)
    };
    if window.is_empty() && !text.is_empty() {
        return None;
    }
    let local = |offset: usize| offset.clamp(window.start, window.end) - window.start;
    Some(ImeSurroundingSnapshot {
        text: text[window.clone()].to_string(),
        cursor: local(cursor),
        anchor: local(anchor),
    })
}

pub fn run_runtime<Program: RuntimeProgram>(
    settings: WindowDescriptor,
) -> Result<(), crate::HostedRunError> {
    crate::run_runtime_scene::<Program>(settings)
}

thread_local! {
    static PENDING_HOST_STORE: RefCell<Option<SharedStore>> = const { RefCell::new(None) };
}

pub(crate) fn take_pending_store() -> SharedStore {
    PENDING_HOST_STORE
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_else(memory_store)
}

/// Same as [`run_runtime`], with a host-injected persistent store.
pub fn run_runtime_with_store<Program: RuntimeProgram>(
    settings: WindowDescriptor,
    store: SharedStore,
) -> Result<(), crate::HostedRunError> {
    PENDING_HOST_STORE.with(|slot| {
        *slot.borrow_mut() = Some(store);
    });
    let _clear_if_unused = PendingHostStoreGuard;
    crate::run_runtime_scene::<Program>(settings)
}

struct PendingHostStoreGuard;

impl Drop for PendingHostStoreGuard {
    fn drop(&mut self) {
        PENDING_HOST_STORE.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pending_store_guard_clears_untaken_slot() {
        super::PENDING_HOST_STORE.with(|slot| {
            *slot.borrow_mut() = Some(nana_ui_core::memory_store());
        });
        {
            let _guard = super::PendingHostStoreGuard;
        }
        super::PENDING_HOST_STORE.with(|slot| {
            assert!(slot.borrow().is_none());
        });
    }

    #[test]
    fn context_clone_accepts_move_only_messages() {
        fn requires_clone<T: Clone>() {}
        requires_clone::<super::RuntimeProgramContext<std::sync::mpsc::Receiver<()>>>();
    }

    use super::{
        IME_SURROUNDING_MAX_BYTES, clip_ime_surrounding, gated_runtime_input_update,
        gated_runtime_window_update, runtime_ime_surrounding, runtime_text_input_request,
    };
    use nana_ui_platform::{InputDisposition, InputEvent, InputModifiers, WindowId};
    use nana_ui_runtime::{AppContext, Dialog, DocumentId, OverlayHost, TextArea};

    #[test]
    fn runtime_ime_request_uses_editability_and_secure_purpose() {
        let document_id = nana_ui_runtime::DocumentId::new(1).unwrap();
        let mut document = nana_ui_scene::RuntimeDocument::new(document_id);
        let input = document
            .context_mut()
            .create_component(
                document_id,
                nana_ui_runtime::TextInput::new("secret").secure(true),
            )
            .unwrap();
        assert!(
            document
                .context_mut()
                .focus_node(document_id, input.stable_id())
                .unwrap()
        );

        let request = runtime_text_input_request(&document);
        assert!(request.enabled);
        assert_eq!(
            request.purpose,
            nana_ui_platform::TextInputPurpose::Password
        );

        document
            .context_mut()
            .update_component(input, |input, _cx| input.read_only = true)
            .unwrap();
        let request = runtime_text_input_request(&document);
        assert!(!request.enabled);
        assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);
    }

    #[test]
    fn runtime_textarea_ime_request_follows_focus_and_normal_purpose() {
        let document_id = nana_ui_runtime::DocumentId::new(1).unwrap();
        let mut document = nana_ui_scene::RuntimeDocument::new(document_id);
        let area = document
            .context_mut()
            .create_component(document_id, TextArea::new("第一行\n第二行"))
            .unwrap();

        let request = runtime_text_input_request(&document);
        assert!(!request.enabled);
        assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);
        assert!(request.cursor_area.is_none());

        assert!(
            document
                .context_mut()
                .focus_node(document_id, area.stable_id())
                .unwrap()
        );
        let request = runtime_text_input_request(&document);
        assert!(request.enabled);
        assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);

        document
            .context_mut()
            .update_component(area, |area, _cx| area.disabled = true)
            .unwrap();
        let request = runtime_text_input_request(&document);
        assert!(!request.enabled);
        assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);
    }

    #[test]
    fn runtime_ime_surrounding_follows_focus_and_skips_password() {
        let document_id = nana_ui_runtime::DocumentId::new(1).unwrap();
        let mut document = nana_ui_scene::RuntimeDocument::new(document_id);
        let input = document
            .context_mut()
            .create_component(document_id, nana_ui_runtime::TextInput::new("NanaUI"))
            .unwrap();
        assert!(runtime_ime_surrounding(&document).is_none());

        assert!(
            document
                .context_mut()
                .focus_node(document_id, input.stable_id())
                .unwrap()
        );
        let surrounding = runtime_ime_surrounding(&document).expect("focused editor");
        assert_eq!(surrounding.text, "NanaUI");
        assert_eq!(surrounding.cursor, "NanaUI".len());
        assert_eq!(surrounding.anchor, "NanaUI".len());

        let password = document
            .context_mut()
            .create_component(
                document_id,
                nana_ui_runtime::TextInput::new("secret").secure(true),
            )
            .unwrap();
        assert!(
            document
                .context_mut()
                .focus_node(document_id, password.stable_id())
                .unwrap()
        );
        assert!(runtime_ime_surrounding(&document).is_none());
    }

    #[test]
    fn clip_ime_surrounding_keeps_a_selection_that_fits_and_uses_the_whole_budget() {
        let text = "a".repeat(IME_SURROUNDING_MAX_BYTES * 3);
        let clip = clip_ime_surrounding(&text, text.len(), text.len() - 3000).unwrap();
        assert_eq!(clip.text.len(), IME_SURROUNDING_MAX_BYTES);
        assert_eq!(
            clip.cursor - clip.anchor,
            3000,
            "the whole selection is reported"
        );
        let short = "ab中cd";
        let clip = clip_ime_surrounding(short, 5, 2).unwrap();
        assert_eq!(
            (clip.text.as_str(), clip.cursor, clip.anchor),
            (short, 5, 2)
        );
    }

    #[test]
    fn clip_ime_surrounding_stays_within_winit_limit() {
        let text = "字".repeat(IME_SURROUNDING_MAX_BYTES);
        let caret = text.len();
        let clip = clip_ime_surrounding(&text, caret, caret).expect("excerpt");
        assert!(clip.text.len() <= IME_SURROUNDING_MAX_BYTES);
        assert!(clip.text.is_char_boundary(clip.cursor));
        assert!(clip.text.is_char_boundary(clip.anchor));
    }

    #[test]
    fn consumed_runtime_input_never_reaches_the_raw_program_handler() {
        let mut called = false;
        let _ = gated_runtime_input_update(
            InputDisposition {
                prevent_default: true,
            },
            WindowId::PRIMARY,
            || {
                called = true;
                Ok(super::RuntimeProgramUpdate::default())
            },
        );
        assert!(!called);

        let _ = gated_runtime_input_update(
            InputDisposition {
                prevent_default: false,
            },
            WindowId::PRIMARY,
            || {
                called = true;
                Ok(super::RuntimeProgramUpdate::default())
            },
        );
        assert!(called);
    }

    #[test]
    fn blocking_overlay_primary_tab_never_reaches_the_raw_program_handler() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Settings"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        context.activate_overlay(host, dialog).unwrap();
        let disposition = crate::RuntimeInputAdapter::default()
            .dispatch(
                &mut context,
                document,
                &InputEvent::Keyboard {
                    pressed: true,
                    key: "Tab".into(),
                    text: None,
                    code: "Tab".into(),
                    repeat: false,
                    modifiers: InputModifiers {
                        control: true,
                        ..InputModifiers::default()
                    },
                },
            )
            .unwrap();

        let mut calls = 0;
        let _ = gated_runtime_input_update(disposition, WindowId::PRIMARY, || {
            calls += 1;
            Ok(super::RuntimeProgramUpdate::default())
        });
        assert_eq!(calls, 0);
    }

    #[test]
    fn gated_window_update_skips_the_raw_handler_only_when_asked() {
        let mut calls = 0;
        let _ = gated_runtime_window_update(true, || {
            calls += 1;
            super::RuntimeProgramUpdate::default()
        });
        assert_eq!(calls, 0);

        let _ = gated_runtime_window_update(false, || {
            calls += 1;
            super::RuntimeProgramUpdate::default()
        });
        assert_eq!(calls, 1);
    }
}

/// One raw input event together with everything the host learned while
/// dispatching it through the Runtime.
#[derive(Debug, Clone, Copy)]
pub struct RoutedInput<'a> {
    /// The event as the platform delivered it.
    pub event: &'a InputEvent,
    /// Topmost interactive node the host hit-tested under the pointer. `Some`
    /// only for [`InputEvent::Pointer`] and [`InputEvent::Wheel`].
    pub pointer_hit: Option<StableNodeId>,
    /// Whether a control consumed the event's default action. The hook still
    /// runs for consumed events so applications can drain pending input; gate
    /// application shortcuts with `disposition.prevent_default` to avoid
    /// handling the same Escape twice.
    pub disposition: nana_ui_platform::InputDisposition,
}

/// Per-target demand; ordinary controls need no periodic clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameDemand {
    #[default]
    OnDemand,
    At(Instant),
    Continuous(std::num::NonZeroU32),
}

#[derive(Default)]
pub(crate) struct FrameSchedule {
    demand: FrameDemand,
    deadline: Option<Instant>,
}

impl FrameSchedule {
    pub(crate) fn due(&self, demand: FrameDemand, now: Instant) -> bool {
        self.armed_deadline(demand, now)
            .is_some_and(|deadline| deadline <= now)
    }

    pub(crate) fn update(&mut self, demand: FrameDemand, now: Instant) -> (bool, Option<Instant>) {
        self.arm(demand, now);
        let due = self.deadline.is_some_and(|deadline| deadline <= now);
        if due {
            self.deadline = match demand {
                FrameDemand::Continuous(fps) => {
                    let hit = self.deadline.expect("due continuous deadline");
                    next_continuous_deadline(hit, now, continuous_period(fps))
                }
                _ => None,
            };
        }
        (due, self.deadline)
    }

    pub(crate) fn arm(&mut self, demand: FrameDemand, now: Instant) -> Option<Instant> {
        if self.demand != demand {
            self.demand = demand;
            self.deadline = Self::fresh_deadline(demand, now);
        }
        self.deadline
    }

    pub(crate) fn defer(&mut self, demand: FrameDemand, now: Instant) -> Option<Instant> {
        self.demand = demand;
        self.deadline = match demand {
            FrameDemand::OnDemand => None,
            FrameDemand::At(at) => {
                let retry = now + PRESENT_RETRY;
                Some(if at > retry { at } else { retry })
            }
            FrameDemand::Continuous(fps) => {
                next_continuous_deadline(now, now, continuous_period(fps))
            }
        };
        self.deadline
    }

    /// Continuous consumes the served tick. At/OnDemand keep a still-due deadline.
    pub(crate) fn advance_served(&mut self, demand: FrameDemand, now: Instant) -> Option<Instant> {
        match demand {
            FrameDemand::Continuous(_) => self.update(demand, now).1,
            _ => self.arm(demand, now),
        }
    }

    fn armed_deadline(&self, demand: FrameDemand, now: Instant) -> Option<Instant> {
        if self.demand == demand {
            self.deadline
        } else {
            Self::fresh_deadline(demand, now)
        }
    }

    fn fresh_deadline(demand: FrameDemand, now: Instant) -> Option<Instant> {
        match demand {
            FrameDemand::OnDemand => None,
            FrameDemand::At(at) => Some(at),
            FrameDemand::Continuous(_) => Some(now),
        }
    }
}

const PRESENT_RETRY: std::time::Duration = std::time::Duration::from_millis(16);

fn continuous_period(fps: std::num::NonZeroU32) -> std::time::Duration {
    std::time::Duration::from_secs_f64(1.0 / f64::from(fps.get()))
        .max(std::time::Duration::from_nanos(1))
}

/// Keep the 1/fps phase. A late frame shortens the next wait instead of
/// pushing `now + period`, so present-to-present stays on the cadence.
/// Missed ticks are skipped so a hitch does not burst catch-up frames.
fn next_continuous_deadline(
    hit: Instant,
    now: Instant,
    period: std::time::Duration,
) -> Option<Instant> {
    let mut next = hit.checked_add(period)?;
    while next <= now {
        next = next.checked_add(period)?;
    }
    Some(next)
}

#[cfg(test)]
mod frame_schedule_tests {
    use super::*;

    #[test]
    fn host_failure_faults_are_limited_to_one_per_second_per_variant() {
        let slot = std::sync::atomic::AtomicU64::new(0);
        assert!(claim_fault_slot(&slot, 1));
        assert!(!claim_fault_slot(&slot, 2));
        assert!(!claim_fault_slot(&slot, 1000));
        assert!(claim_fault_slot(&slot, 1001));
        assert!(!claim_fault_slot(&slot, 1500));
        assert!(claim_fault_slot(&slot, 2500));
    }
    use std::time::Duration;

    fn fps(n: u32) -> FrameDemand {
        FrameDemand::Continuous(std::num::NonZeroU32::new(n).unwrap())
    }

    #[test]
    fn continuous_skips_missed_ticks_and_deadline_fires_once() {
        let now = Instant::now();
        let mut schedule = FrameSchedule::default();
        assert!(schedule.update(fps(120), now).0);
        assert!(!schedule.update(fps(120), now).0);
        let later = now + Duration::from_secs(1);
        let (due, next) = schedule.update(fps(120), later);
        assert!(due);
        assert!(next.unwrap() > later);
        assert!(!schedule.update(fps(120), later).0);
        assert!(schedule.update(FrameDemand::At(later), later).0);
        assert!(!schedule.update(FrameDemand::At(later), later).0);
        assert_eq!(schedule.update(FrameDemand::OnDemand, later), (false, None));
    }

    #[test]
    fn due_peek_does_not_consume() {
        let now = Instant::now();
        let mut schedule = FrameSchedule::default();
        assert!(schedule.due(fps(60), now));
        assert!(schedule.due(fps(60), now));
        assert!(schedule.update(fps(60), now).0);
        assert!(!schedule.due(fps(60), now));
    }

    #[test]
    fn at_demand_reschedules_when_the_instant_moves() {
        let t0 = Instant::now();
        let mut schedule = FrameSchedule::default();
        assert!(schedule.update(FrameDemand::At(t0), t0).0);
        assert_eq!(schedule.update(FrameDemand::At(t0), t0), (false, None));
        let t1 = t0 + Duration::from_millis(16);
        let (due, next) = schedule.update(FrameDemand::At(t1), t0);
        assert!(!due);
        assert_eq!(next, Some(t1));
    }

    #[test]
    fn advance_served_keeps_a_due_at_armed() {
        let t0 = Instant::now();
        let mut schedule = FrameSchedule::default();
        assert!(schedule.due(FrameDemand::At(t0), t0));
        assert_eq!(schedule.advance_served(FrameDemand::At(t0), t0), Some(t0));
        assert!(schedule.due(FrameDemand::At(t0), t0));
        let t1 = t0 + Duration::from_millis(16);
        assert_eq!(schedule.advance_served(FrameDemand::At(t1), t0), Some(t1));
        assert!(!schedule.due(FrameDemand::At(t1), t0));
    }

    #[test]
    fn advance_served_paces_continuous() {
        let now = Instant::now();
        let mut schedule = FrameSchedule::default();
        assert!(schedule.due(fps(60), now));
        let next = schedule.advance_served(fps(60), now);
        assert!(next.unwrap() > now);
        assert!(!schedule.due(fps(60), now));
        schedule.arm(fps(60), now);
        assert!(!schedule.due(fps(60), now));
    }

    #[test]
    fn defer_retries_after_backoff() {
        let t0 = Instant::now();
        let mut schedule = FrameSchedule::default();
        assert!(schedule.update(FrameDemand::At(t0), t0).0);
        let next = schedule.defer(FrameDemand::At(t0), t0).expect("retry");
        assert!(next > t0);
        assert!(!schedule.due(FrameDemand::At(t0), t0));
        assert!(schedule.due(FrameDemand::At(t0), next));
        let next = schedule.defer(fps(60), t0).expect("period");
        assert!(next > t0);
        assert!(!schedule.due(fps(60), t0));
    }

    #[test]
    fn continuous_keeps_phase_when_a_frame_runs_late() {
        let start = Instant::now();
        let mut schedule = FrameSchedule::default();
        assert!(schedule.update(fps(120), start).0);
        let first_deadline = schedule.update(fps(120), start).1.expect("period deadline");
        let period = first_deadline.saturating_duration_since(start);
        let late = first_deadline + Duration::from_micros(250);
        let (due, next) = schedule.update(fps(120), late);
        assert!(due);
        let next = next.expect("phase-locked deadline");
        assert_eq!(next, first_deadline + period);
        assert!(next > late);
        assert!(next < late + period);
    }
}

#[cfg(test)]
mod terminal_host_tests {
    use super::*;

    #[test]
    fn terminal_requests_ime_at_its_grid_cursor() {
        let id = nana_ui_runtime::DocumentId::new(1).unwrap();
        let mut document = nana_ui_scene::RuntimeDocument::new(id);
        let mut screen = nana_ui_runtime::TerminalScreen::blank(10, 4);
        screen.cursor = Some(nana_ui_runtime::TerminalCursor {
            position: nana_ui_runtime::TerminalPosition { row: 2, column: 3 },
            shape: nana_ui_runtime::TerminalCursorShape::Bar,
            visible: true,
        });
        let terminal = document
            .context_mut()
            .create_component(id, nana_ui_runtime::TerminalView::new(screen))
            .unwrap();
        let mut mutations = nana_ui_runtime::MutationQueue::new();
        mutations.write_layout(
            terminal.stable_id(),
            nana_ui_runtime::LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 80.0,
                height: 72.0,
            },
        );
        document.context_mut().commit_mutations(mutations).unwrap();
        document
            .context_mut()
            .focus_node(id, terminal.stable_id())
            .unwrap();
        let request = runtime_text_input_request(&document);
        assert!(request.enabled);
        assert_eq!(
            request.cursor_area,
            Some(nana_ui_core::LogicalRect::new(34.0, 56.0, 8.0, 18.0))
        );
        assert!(runtime_ime_surrounding(&document).is_none());
        document
            .context_mut()
            .update_component(terminal, |view, _| view.read_only = true)
            .unwrap();
        assert!(!runtime_text_input_request(&document).enabled);
        assert!(document.context().focused_terminal(id).is_some());
    }
}
