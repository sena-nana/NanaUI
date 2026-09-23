//! The standalone host's startup (Issue #225): the window and its splash
//! first, the device on a thread of its own, then the program and the
//! handoff.
//!
//! The ordering rules live in [`StartupCoordinator`], kept off the window and
//! GPU calls so each of them can be tested on a machine with no display. The
//! host feeds it what happened — the program became ready, a takeover was
//! asked for or withdrawn, a frame was presented — and asks it two questions:
//! may the primary window present yet, and does this presented frame end the
//! splash. Frames that were skipped, retried or failed never reach it: the
//! host only reports a frame after its `present` succeeded.

use std::sync::atomic::{AtomicBool, Ordering};

use nana_diagnostics::framework::host;
use nana_window::{NativeSplash, SplashHandoff};

use super::*;
use crate::hosted_context::{AcquiredDevice, DeviceRequest, PendingPrimarySurface};
use crate::presentation::{
    CompositionAvailability, ResolvedSurfaceTarget, resolve_window_surface_target,
    window_surface_request,
};
use crate::startup::{
    SplashOutcome, SplashSkip, StartupError, StartupHandle, StartupOptions, StartupPhase,
    StartupRequest, StartupStatus, StartupTakeover, StartupTicket,
};

/// How long the Windows handoff waits between checks that the takeover
/// frame's GPU work has completed. Only while that one frame is in flight.
const LATCH_POLL: Duration = Duration::from_millis(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StartupCoordinator {
    phase: StartupPhase,
    /// Current ticket generation. A cancel retires it.
    generation: u64,
    /// A native splash covers the primary window.
    splash: bool,
    /// The primary window's flush sequence when the takeover was asked for.
    /// Only a frame flushed after it shows the content the request named.
    requested_after: Option<u64>,
    /// The takeover frame has been presented; only the splash's removal is
    /// left. Nothing can be withdrawn any more.
    committed: bool,
}

impl StartupCoordinator {
    pub(super) const fn new(splash: bool) -> Self {
        Self {
            phase: StartupPhase::Starting,
            generation: 1,
            splash,
            requested_after: None,
            committed: false,
        }
    }

    pub(super) const fn phase(&self) -> StartupPhase {
        self.phase
    }

    pub(super) const fn ticket(&self) -> Option<StartupTicket> {
        match self.phase {
            StartupPhase::UiReady | StartupPhase::TakeoverRequested => Some(StartupTicket {
                generation: self.generation,
            }),
            StartupPhase::Starting | StartupPhase::HandedOff => None,
        }
    }

    /// The program exists. `flush` is the primary window's flush sequence
    /// now. Returns whether this also requested the takeover.
    pub(super) fn ui_ready(&mut self, policy: StartupTakeover, flush: u64) -> bool {
        if self.phase != StartupPhase::Starting {
            return false;
        }
        self.phase = StartupPhase::UiReady;
        match policy {
            StartupTakeover::Immediate => {
                self.phase = StartupPhase::TakeoverRequested;
                self.requested_after = Some(flush);
                true
            }
            StartupTakeover::Deferred => false,
        }
    }

    pub(super) fn request(
        &mut self,
        ticket: StartupTicket,
        flush: u64,
    ) -> Result<(), StartupError> {
        self.check(ticket)?;
        if self.phase == StartupPhase::UiReady {
            self.phase = StartupPhase::TakeoverRequested;
            self.requested_after = Some(flush);
        }
        // A repeated request keeps the first target: it named content that is
        // already in every later frame.
        Ok(())
    }

    pub(super) fn cancel(&mut self, ticket: StartupTicket) -> Result<(), StartupError> {
        self.check(ticket)?;
        self.generation += 1;
        self.phase = StartupPhase::UiReady;
        self.requested_after = None;
        Ok(())
    }

    fn check(&self, ticket: StartupTicket) -> Result<(), StartupError> {
        match self.phase {
            StartupPhase::HandedOff => Err(StartupError::AlreadyHandedOff),
            _ if self.committed => Err(StartupError::AlreadyHandedOff),
            StartupPhase::Starting => Err(StartupError::StaleTicket),
            _ if ticket.generation != self.generation => Err(StartupError::StaleTicket),
            _ => Ok(()),
        }
    }

    /// Whether `id` must not present yet. Only the primary window under a
    /// splash waits, and only until something asked to take over: until then
    /// nothing it drew could be seen, and the window is already on screen, so
    /// holding it back cannot keep it from ever presenting.
    pub(super) fn holds_presents(&self, id: WindowId) -> bool {
        self.splash
            && id == WindowId::PRIMARY
            && matches!(self.phase, StartupPhase::Starting | StartupPhase::UiReady)
    }

    /// Whether a presented frame of `id`, flushed at `flush`, ends the
    /// startup. Only a requested takeover ends it, splash or not, so a
    /// program that defers behaves the same on a platform without a splash:
    /// its ticket stays valid until it asks.
    pub(super) fn completes_with(&self, id: WindowId, flush: u64) -> bool {
        id == WindowId::PRIMARY
            && !self.committed
            && self.phase == StartupPhase::TakeoverRequested
            && self.requested_after.is_some_and(|after| flush > after)
    }

    /// The takeover frame was presented; the handoff finishes once the
    /// compositor has it. A cancel arriving meanwhile is refused.
    pub(super) fn frame_committed(&mut self) {
        self.committed = true;
    }

    pub(super) fn handed_off(&mut self) {
        self.phase = StartupPhase::HandedOff;
        self.requested_after = None;
    }
}

/// What `run_runtime_scene` hands the event loop before it has a window.
pub(super) struct LoadingStartup<Message> {
    pub(super) proxy: EventLoopProxy,
    pub(super) message_tx: Sender<Message>,
    pub(super) message_rx: Receiver<Message>,
    pub(super) settings: WindowDescriptor,
    pub(super) startup_failure: Arc<Mutex<Option<String>>>,
    pub(super) options: StartupOptions,
    pub(super) entry: Instant,
}

/// What the startup thread sends back: a device for the surface, and the
/// scene painter built on it, so the pipelines are not compiled on the event
/// thread either.
pub(super) struct DeviceStart {
    device: Result<AcquiredDevice, String>,
    painter: Option<SceneWgpuPainter>,
}

/// One presentation target being tried. Fields drop in declaration order:
/// the splash comes off before the window it covers goes.
struct Attempt {
    splash: Option<NativeSplash>,
    #[cfg(not(target_os = "android"))]
    accessibility: Option<HostedAccessibility>,
    surface: PendingPrimarySurface,
    provisional: PendingNativeWindow,
    window: Arc<dyn winit::window::Window>,
    requested_material: crate::MaterialEffect,
    applied_material: MaterialOutcome,
    device: Receiver<DeviceStart>,
}

/// The primary window exists — on screen with its splash, when there is one —
/// and its device is being requested off the event thread. The event loop
/// keeps turning meanwhile: closing the window cancels the startup.
pub(super) struct PendingStartup<Message> {
    channels: StartupChannels<Message>,
    settings: WindowDescriptor,
    store: SharedStore,
    options: StartupOptions,
    policy: crate::GpuBackendPolicy,
    target: ResolvedSurfaceTarget,
    /// Why the composed target was given up, for a plain attempt that fails too.
    composed_error: Option<String>,
    handle: StartupHandle,
    longest_block: Duration,
    icons: Option<Receiver<SceneIcons>>,
    attempt: Option<Attempt>,
}

pub(super) enum StartupStep<Program: RuntimeProgram> {
    Ready(Box<WindowManager<Program>>),
    /// The composed target failed; the plain one is being tried.
    Retry(Box<PendingStartup<Program::Message>>),
}

impl<Message: Send + 'static> PendingStartup<Message> {
    pub(super) fn begin<Program: RuntimeProgram<Message = Message>>(
        event_loop: &dyn ActiveEventLoop,
        loading: LoadingStartup<Message>,
    ) -> Result<Self, String> {
        let LoadingStartup {
            proxy,
            message_tx,
            message_rx,
            mut settings,
            startup_failure,
            options,
            entry,
        } = loading;
        let store = prepare_primary_descriptor(&mut settings)?;
        let policy = Program::gpu_backend_policy();
        // Two separate questions, in order. First: can this process present
        // through a platform compositor at all? That is the GPU backend's
        // answer, it is process-wide, and it has to be settled before any
        // window exists because the redirection bitmap is a creation-time
        // flag. Second: does *this* window want that path? Every other window
        // asks it again for itself.
        let mut bootstrap = gpu_bootstrap(policy, None);
        let requested = window_surface_request(
            settings.surface,
            window_wants_transparent_surface(settings.transparent, crate::MaterialEffect::Solid),
            policy,
        );
        let target = resolve_window_surface_target(
            requested,
            settings.surface.requires_composition(),
            composition_availability(&bootstrap),
        );
        if let Some(reason) = target.forbidden_fallback() {
            // The application said it would rather not start than present
            // this window another way.
            return Err(format!(
                "window requires a platform compositor surface: {}",
                reason.label()
            ));
        }
        if options.splash.is_some() {
            // Font discovery is independent of the device; with the window
            // already up, it overlaps the device request instead of following
            // it. Only with a splash: without one the engine is built where it
            // always was, after `initialize`.
            let _ = std::thread::Builder::new()
                .name("nana-startup-fonts".into())
                .spawn(|| drop(crate::text_engine::nana_text_engine()));
        }
        let icons = {
            let (sender, receiver) = mpsc::channel();
            let per_window = settings.icon.clone();
            std::thread::Builder::new()
                .name("nana-startup-icons".into())
                .spawn({
                    let proxy = proxy.clone();
                    move || {
                        if sender
                            .send(SceneIcons::render(per_window.as_ref(), true))
                            .is_ok()
                        {
                            proxy.wake_up();
                        }
                    }
                })
                .ok()
                .map(|_| receiver)
        };
        let mut pending = Self {
            channels: StartupChannels {
                proxy,
                message_tx,
                message_rx,
                startup_failure,
            },
            settings,
            store,
            options,
            policy,
            target,
            composed_error: None,
            handle: StartupHandle::new(entry, SplashOutcome::Skipped(SplashSkip::NotConfigured)),
            longest_block: Duration::ZERO,
            icons,
            attempt: None,
        };
        // The probe's instance is narrowed to the composition backend, and
        // that narrowing is the process-wide policy taking effect. It is
        // dropped only once composition has been given up on.
        let instance = target
            .fallback
            .is_none()
            .then(|| bootstrap.take_instance())
            .flatten();
        pending.start(event_loop, instance)?;
        Ok(pending)
    }

    pub(super) fn startup_failure(&self) -> &Arc<Mutex<Option<String>>> {
        &self.channels.startup_failure
    }

    pub(super) fn owns(&self, id: winit::window::WindowId) -> bool {
        self.attempt
            .as_ref()
            .is_some_and(|attempt| attempt.window.id() == id)
    }

    pub(super) fn note_block(&mut self, elapsed: Duration) {
        self.longest_block = self.longest_block.max(elapsed);
    }

    /// The window's backing scale changed while the device is requested.
    pub(super) fn rescale_splash(&mut self, id: winit::window::WindowId, scale: f64) {
        if let Some(attempt) = self.attempt.as_mut()
            && attempt.window.id() == id
            && let Some(splash) = attempt.splash.as_mut()
        {
            splash.set_scale_factor(scale);
        }
    }

    /// The startup was cancelled. The device thread may still hold the
    /// window through its surface, so it is hidden here rather than left for
    /// the last reference to take down; the splash comes off with the drop.
    pub(super) fn cancel(self) {
        if let Some(attempt) = self.attempt.as_ref() {
            attempt.window.set_visible(false);
        }
    }

    /// The startup thread's result, once it has sent one.
    pub(super) fn take_device(&mut self) -> Option<DeviceStart> {
        match self.attempt.as_ref()?.device.try_recv() {
            Ok(start) => Some(start),
            Err(mpsc::TryRecvError::Empty) => None,
            // The thread ended without sending: it panicked.
            Err(mpsc::TryRecvError::Disconnected) => Some(DeviceStart {
                device: Err("the GPU startup thread stopped without a device".into()),
                painter: None,
            }),
        }
    }

    /// Starts the current target, falling back to the plain one while there
    /// is one to fall back to.
    fn start(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        mut instance: Option<wgpu::Instance>,
    ) -> Result<(), String> {
        loop {
            match self.start_attempt(event_loop, instance.take()) {
                Ok(()) => return Ok(()),
                Err(error) => self.fall_back(error)?,
            }
        }
    }

    /// Drops the current attempt — splash, window and all — and moves to the
    /// next target, or gives up with the whole story.
    fn fall_back(&mut self, error: String) -> Result<(), String> {
        self.attempt = None;
        match next_bootstrap_attempt(self.target) {
            Some(next) => {
                self.composed_error = Some(error);
                self.target = next;
                Ok(())
            }
            None => Err(match self.composed_error.take() {
                Some(composed) => format!("{error} (after composition failed: {composed})"),
                None => error,
            }),
        }
    }

    fn start_attempt(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        instance: Option<wgpu::Instance>,
    ) -> Result<(), String> {
        let target = self.target.resolved;
        let (window, provisional, requested_material, applied_material) = create_primary_window(
            event_loop,
            &self.settings,
            target,
            crate::ThemeMode::default(),
            crate::MaterialEffect::Solid,
        )?;
        // The accessibility adapter belongs to the window before it is first
        // shown, which with a splash is now.
        #[cfg(not(target_os = "android"))]
        let accessibility = Some(HostedAccessibility::new(
            Arc::clone(&window),
            true,
            window.scale_factor() as f32,
        ));
        let (splash, outcome) = self.show_splash(window.as_ref(), target);
        let work = splash.as_ref().map(NativeSplash::work).unwrap_or_default();
        let committed = splash.is_some().then(|| self.handle.elapsed());
        if splash.is_some() {
            windows::set_native_visible(window.as_ref(), true, self.settings.focus_on_show);
        }
        nana_diagnostics::event!(host::SPLASH_OUTCOME, outcome = outcome.code());
        if let Some(at) = committed {
            startup_phase_event(1, at);
        }
        self.handle.update(|status| {
            status.splash = outcome;
            status.timeline.splash_committed = committed;
            status.work.splash = work;
        });
        let want_transparent = requested_material.wants_transparent_surface();
        let (surface, request) = PendingPrimarySurface::begin(
            Arc::clone(&window),
            wgpu::Features::empty(),
            want_transparent,
            surface_mode_for(target),
            instance,
        )
        .map_err(|error| error.to_string())?;
        let device = spawn_device_request(request, self.channels.proxy.clone())?;
        self.handle
            .update(|status| status.work.devices_requested += 1);
        self.attempt = Some(Attempt {
            splash,
            #[cfg(not(target_os = "android"))]
            accessibility,
            surface,
            provisional,
            window,
            requested_material,
            applied_material,
            device,
        });
        Ok(())
    }

    fn show_splash(
        &self,
        window: &dyn winit::window::Window,
        target: WindowSurfaceTarget,
    ) -> (Option<NativeSplash>, SplashOutcome) {
        let Some(spec) = self.options.splash else {
            return (None, SplashOutcome::Skipped(SplashSkip::NotConfigured));
        };
        let skip = if !self.settings.visible {
            Some(SplashSkip::HiddenStart)
        } else if target.composed() {
            Some(SplashSkip::CompositionTarget)
        } else if !NativeSplash::platform_supported() {
            Some(SplashSkip::PlatformUnsupported)
        } else {
            None
        };
        if let Some(skip) = skip {
            return (None, SplashOutcome::Skipped(skip));
        }
        let theme = match window.theme() {
            Some(WinitTheme::Light) => crate::ThemeMode::Light,
            Some(WinitTheme::Dark) => crate::ThemeMode::Dark,
            None => crate::ThemeMode::default(),
        };
        let background = theme.palette().background;
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        NativeSplash::show(
            window,
            &spec,
            FallbackColor::rgba(
                channel(background.r),
                channel(background.g),
                channel(background.b),
                channel(background.a),
            ),
            nana_window::system_reduced_motion().unwrap_or(false),
        )
    }

    /// Binds the surface to the device the startup thread produced and runs
    /// the rest of the startup on this thread, or moves to the next target.
    pub(super) fn finish<Program: RuntimeProgram<Message = Message>>(
        mut self,
        event_loop: &dyn ActiveEventLoop,
        start: DeviceStart,
    ) -> Result<StartupStep<Program>, String> {
        let Attempt {
            splash,
            #[cfg(not(target_os = "android"))]
            accessibility,
            surface,
            provisional,
            window,
            requested_material,
            applied_material,
            device: _,
        } = self
            .attempt
            .take()
            .expect("a device result belongs to an attempt");
        let composed = self.target.resolved.composed();
        let bound = start
            .device
            .and_then(|device| surface.finish(device).map_err(|error| error.to_string()))
            .and_then(|context| match composition_fault_injection() {
                // The composed path's failure branch is not reachable from a
                // test that has no way to make DirectComposition fail, so the
                // acceptance probe asks for it explicitly.
                Some(reason) if composed => Err(reason),
                _ => Ok(context),
            });
        let context = match bound {
            Ok(context) => context,
            Err(error) => {
                drop(splash);
                drop(provisional);
                self.fall_back(error)?;
                self.start(event_loop, None)?;
                return Ok(StartupStep::Retry(Box::new(self)));
            }
        };
        provisional.keep();
        if let Some(reason) = self.target.fallback {
            eprintln!(
                "nana window surface: {}; presenting through the plain window path instead",
                reason.label()
            );
            nana_diagnostics::set_session_info("window.presentation_fallback", reason.label());
        }
        let (graphics, surface) = context.into_parts();
        // A composed target that could not be built for this window will not
        // build for another, so the failure narrows the whole process.
        let composition = match self.target.fallback {
            Some(_) => CompositionAvailability::Unavailable,
            None => {
                CompositionAvailability::for_backend(self.policy, graphics.adapter_info().backend)
            }
        };
        let shown_early = splash.is_some();
        let startup = HostStartup::new(self.handle, splash, shown_early, self.longest_block);
        complete_startup::<Program>(
            event_loop,
            self.channels,
            self.settings,
            self.store,
            false,
            PrimaryStart {
                bootstrap: PrimaryBootstrap {
                    window,
                    graphics,
                    surface,
                    target: self.target,
                    composition,
                    requested_material,
                    applied_material,
                },
                #[cfg(not(target_os = "android"))]
                accessibility,
                painter: start.painter,
                icons: self.icons,
                startup,
            },
        )
        .map(|ready| StartupStep::Ready(Box::new(ready)))
    }
}

/// Requests the device on a thread of its own and wakes the event loop when
/// it has one. A platform device request cannot be cancelled once it has
/// started; a startup cancelled meanwhile simply never takes the result.
fn spawn_device_request(
    request: DeviceRequest,
    proxy: EventLoopProxy,
) -> Result<Receiver<DeviceStart>, String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("nana-startup-gpu".into())
        .spawn(move || {
            if let Some(delay) = startup_fault_delay("NANA_STARTUP_GPU_DELAY_MS") {
                std::thread::sleep(delay);
            }
            let device = if startup_fault_flag("NANA_STARTUP_GPU_FAIL") {
                drop(request);
                Err("GPU initialization failure requested by NANA_STARTUP_GPU_FAIL".into())
            } else {
                pollster::block_on(request.acquire()).map_err(|error| error.to_string())
            };
            let painter = device.as_ref().ok().map(|device| {
                let resources = device.resources();
                SceneWgpuPainter::new(resources.device(), resources.queue(), device.format())
            });
            if let Err(unsent) = sender.send(DeviceStart { device, painter }) {
                // The host went away. The surface holds a reference to its
                // window, which must not be released off the window's thread:
                // keep it, once, for the rest of the process.
                std::mem::forget(unsent);
                return;
            }
            proxy.wake_up();
        })
        .map_err(|error| format!("failed to start the GPU startup thread: {error}"))?;
    Ok(receiver)
}

/// Startup fault injection for the acceptance probe (`startup-splash`), in the
/// manner of `NANA_FORCE_COMPOSITION_FAILURE`: nothing in the product sets it.
fn startup_fault_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0")
}

fn startup_fault_delay(name: &str) -> Option<Duration> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|&millis| millis > 0)
        .map(Duration::from_millis)
}

fn startup_phase_event(phase: u64, at: Duration) {
    nana_diagnostics::event!(
        host::STARTUP_PHASE,
        phase = phase,
        elapsed_ns = u64::try_from(at.as_nanos()).unwrap_or(u64::MAX)
    );
}

/// The host's side of a startup once the program exists: the coordinator,
/// the splash it owns until handoff, and the record programs read.
pub(super) struct HostStartup {
    pub(super) handle: StartupHandle,
    coordinator: StartupCoordinator,
    splash: Option<NativeSplash>,
    shown_early: bool,
    /// Windows: set by the queue once the takeover frame's GPU work has
    /// completed; the splash comes off after the next compositor pass.
    latch: Option<Arc<AtomicBool>>,
    /// Flushes of the primary document so far.
    primary_flushes: u64,
    /// An `Immediate` takeover waiting for the startup messages: the frame
    /// that removes the splash has to show their effects too.
    auto_takeover: bool,
    longest_block: Duration,
    /// When the handoff completed: a callback that started before it still
    /// belongs to the startup, and the one that performed the handoff — often
    /// the longest, with the first frame in it — is exactly such a callback.
    handed_off_at: Option<Instant>,
}

impl HostStartup {
    fn new(
        handle: StartupHandle,
        splash: Option<NativeSplash>,
        shown_early: bool,
        longest_block: Duration,
    ) -> Self {
        Self {
            coordinator: StartupCoordinator::new(splash.is_some()),
            handle,
            splash,
            shown_early,
            latch: None,
            primary_flushes: 0,
            auto_takeover: false,
            longest_block,
            handed_off_at: None,
        }
    }

    /// An embedded host's record: nothing to hand off, nothing to measure.
    pub(super) fn settled(outcome: SplashOutcome) -> Self {
        let mut coordinator = StartupCoordinator::new(false);
        coordinator.handed_off();
        Self {
            handle: StartupHandle::settled(outcome),
            coordinator,
            splash: None,
            shown_early: false,
            latch: None,
            primary_flushes: 0,
            auto_takeover: false,
            longest_block: Duration::ZERO,
            handed_off_at: Some(Instant::now()),
        }
    }

    pub(super) const fn shown_early(&self) -> bool {
        self.shown_early
    }

    const fn measuring(&self) -> bool {
        !matches!(self.coordinator.phase(), StartupPhase::HandedOff)
    }

    /// Whether a callback that started at `started` is part of the startup.
    fn measures(&self, started: Instant) -> bool {
        self.handed_off_at.is_none_or(|at| started <= at)
    }

    /// `RuntimeProgram::initialize` is about to be called.
    pub(super) fn ui_ready_begins(&mut self) {
        if !self.measuring() {
            return;
        }
        let at = self.handle.elapsed();
        startup_phase_event(2, at);
        self.handle
            .update(|status| status.timeline.ui_ready = Some(at));
    }

    fn publish(&self, change: impl FnOnce(&mut StartupStatus)) {
        let phase = self.coordinator.phase();
        let ticket = self.coordinator.ticket();
        self.handle.update(|status| {
            status.phase = phase;
            status.ticket = ticket;
            change(status);
        });
    }
}

impl<Program: RuntimeProgram> WindowManager<Program> {
    /// `initialize` returned; the program's takeover policy is known.
    ///
    /// With startup messages still queued, an `Immediate` takeover is asked
    /// for once they have been applied ([`Self::drain_startup_messages`]).
    pub(super) fn startup_ui_ready(&mut self, policy: StartupTakeover) {
        if self.startup.coordinator.phase() != StartupPhase::Starting {
            return;
        }
        let policy = if policy == StartupTakeover::Immediate && !self.startup_messages.is_empty() {
            self.startup.auto_takeover = true;
            StartupTakeover::Deferred
        } else {
            policy
        };
        let requested = self
            .startup
            .coordinator
            .ui_ready(policy, self.startup.primary_flushes);
        let at = requested.then(|| self.startup.handle.elapsed());
        if let Some(at) = at {
            startup_phase_event(3, at);
        }
        self.startup.publish(|status| {
            if at.is_some() {
                status.timeline.takeover_requested = at;
            }
        });
    }

    /// Whether `id` must not present yet; see [`StartupCoordinator::holds_presents`].
    pub(super) fn startup_holds(&self, id: WindowId) -> bool {
        self.startup.coordinator.holds_presents(id)
    }

    /// Counts a settled flush of `id`'s document and returns the primary
    /// window's flush sequence.
    pub(super) fn note_startup_flush(&mut self, id: WindowId) -> u64 {
        if id == WindowId::PRIMARY {
            self.startup.primary_flushes += 1;
        }
        self.startup.primary_flushes
    }

    /// Whether the frame about to be presented for `id` ends the startup, and
    /// if so, readies the window's presentation for the splash's handoff.
    pub(super) fn prepare_startup_frame(&mut self, id: WindowId, flush: u64) -> bool {
        if !self.startup.coordinator.completes_with(id, flush) {
            return false;
        }
        // The drawable and the splash's removal have to land in one Core
        // Animation commit, which only a transaction-mode present gives. It
        // has to be set before the drawable is acquired; the pin is released
        // by the ordinary idle unpin once the turn is over.
        #[cfg(target_os = "macos")]
        if self
            .startup
            .splash
            .as_ref()
            .is_some_and(|splash| splash.handoff() == SplashHandoff::SameTransaction)
        {
            self.pin_present_transaction(id);
        }
        true
    }

    /// The frame [`Self::prepare_startup_frame`] picked was presented.
    pub(super) fn startup_frame_presented(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.startup.coordinator.frame_committed();
        let at = self.startup.handle.elapsed();
        startup_phase_event(4, at);
        self.startup
            .handle
            .update(|status| status.timeline.first_frame_submitted = Some(at));
        match self.startup.splash.as_ref().map(NativeSplash::handoff) {
            Some(SplashHandoff::AfterCompositorFlush) => {
                let latch = Arc::new(AtomicBool::new(false));
                let done = Arc::clone(&latch);
                let proxy = self.proxy.clone();
                self.graphics
                    .resources()
                    .queue()
                    .on_submitted_work_done(move || {
                        done.store(true, Ordering::Release);
                        proxy.wake_up();
                    });
                self.startup.latch = Some(latch);
            }
            Some(SplashHandoff::SameTransaction) | None => self.startup_handed_off(event_loop),
        }
    }

    /// Windows: finishes the handoff once the takeover frame's GPU work is
    /// done. Returns when to look again while it is not.
    pub(super) fn poll_startup_latch(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
    ) -> Option<Instant> {
        let latch = self.startup.latch.as_ref()?;
        if !latch.load(Ordering::Acquire) {
            let _ = self
                .graphics
                .resources()
                .device()
                .poll(wgpu::PollType::Poll);
        }
        if latch.load(Ordering::Acquire) {
            self.startup.latch = None;
            self.startup_handed_off(event_loop);
            None
        } else {
            Some(Instant::now() + LATCH_POLL)
        }
    }

    fn startup_handed_off(&mut self, event_loop: &dyn ActiveEventLoop) {
        let at = self.startup.handle.elapsed();
        let splash = self.startup.splash.take().map(NativeSplash::remove);
        let released = splash.map(|_| self.startup.handle.elapsed());
        self.startup.coordinator.handed_off();
        self.startup.handed_off_at = Some(Instant::now());
        startup_phase_event(5, at);
        if let Some(released) = released {
            startup_phase_event(6, released);
        }
        let longest = self.startup.longest_block;
        nana_diagnostics::metric!(host::STARTUP_LONGEST_BLOCK_NS, longest);
        self.startup.publish(|status| {
            status.timeline.handoff_completed = Some(at);
            if released.is_some() {
                status.timeline.splash_released = released;
            }
            if let Some(work) = splash {
                status.work.splash = work;
            }
            status.work.longest_event_thread_block = longest;
        });
        self.notify_startup_changed(event_loop);
    }

    /// Applies the startup messages `initialize` returned, in batches that let
    /// the loop turn, then asks for the `Immediate` takeover they held back.
    pub(super) fn drain_startup_messages(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !self.startup_messages.is_empty() {
            let more = schedule::drain_host_batch(
                || {
                    if self.shutting_down || event_loop.exiting() {
                        return false;
                    }
                    let Some(message) = self.startup_messages.pop_front() else {
                        return false;
                    };
                    self.process_message(event_loop, message);
                    true
                },
                Instant::now,
            );
            if more && !self.startup_messages.is_empty() {
                self.host_work.wake();
                return;
            }
            self.startup_messages.clear();
        }
        if std::mem::take(&mut self.startup.auto_takeover)
            && let Some(ticket) = self.startup.coordinator.ticket()
            && self
                .startup
                .coordinator
                .request(ticket, self.startup.primary_flushes)
                .is_ok()
        {
            self.takeover_requested();
        }
    }

    /// The coordinator accepted a takeover: record it and draw the frame.
    fn takeover_requested(&mut self) {
        let at = self.startup.handle.elapsed();
        startup_phase_event(3, at);
        self.startup
            .publish(|status| status.timeline.takeover_requested = Some(at));
        self.request_redraw(WindowId::PRIMARY);
    }

    /// Applies takeover requests made through [`crate::StartupHandle`].
    pub(super) fn process_startup_requests(&mut self, event_loop: &dyn ActiveEventLoop) {
        for request in self.startup.handle.take_requests() {
            let before = self.startup.coordinator.phase();
            let applied = match request {
                StartupRequest::TakeOver(ticket) => self
                    .startup
                    .coordinator
                    .request(ticket, self.startup.primary_flushes),
                StartupRequest::Cancel(ticket) => self.startup.coordinator.cancel(ticket),
            };
            let after = self.startup.coordinator.phase();
            if applied.is_err()
                || (before == after && matches!(request, StartupRequest::TakeOver(_)))
            {
                continue;
            }
            match request {
                StartupRequest::TakeOver(_) => self.takeover_requested(),
                StartupRequest::Cancel(_) => {
                    // The program withdrew; nothing is requested on its behalf.
                    self.startup.auto_takeover = false;
                    self.startup
                        .publish(|status| status.timeline.takeover_requested = None);
                }
            }
            self.notify_startup_changed(event_loop);
        }
    }

    fn notify_startup_changed(&mut self, event_loop: &dyn ActiveEventLoop) {
        let status = self.startup.handle.status();
        let update = self.program.startup_changed(&status, &self.context());
        self.apply_update(event_loop, update, None);
    }

    /// The primary window is closing: nothing is left for the splash to
    /// cover, and it has to come off before the window goes.
    pub(super) fn release_startup_splash(&mut self) {
        self.startup.latch = None;
        let Some(splash) = self.startup.splash.take() else {
            return;
        };
        let work = splash.discard();
        let released = self.startup.handle.elapsed();
        startup_phase_event(6, released);
        self.startup.handle.update(|status| {
            status.timeline.splash_released = Some(released);
            status.work.splash = work;
        });
    }

    /// The primary window's backing scale changed while its splash is up.
    pub(super) fn rescale_startup_splash(&mut self, id: WindowId, scale: f64) {
        if id == WindowId::PRIMARY
            && let Some(splash) = self.startup.splash.as_mut()
        {
            splash.set_scale_factor(scale);
        }
    }

    pub(super) fn note_startup_block(&mut self, started: Instant) {
        let elapsed = started.elapsed();
        if self.startup.measures(started) && elapsed > self.startup.longest_block {
            self.startup.longest_block = elapsed;
            self.startup
                .handle
                .update(|status| status.work.longest_event_thread_block = elapsed);
            if !self.startup.measuring() {
                nana_diagnostics::metric!(host::STARTUP_LONGEST_BLOCK_NS, elapsed);
            }
        }
    }

    /// Applies the primary window's icons once the startup thread has
    /// rendered them.
    pub(super) fn apply_pending_icons(&mut self) {
        let Some(icons) = self.pending_icons.as_ref() else {
            return;
        };
        let rendered = match icons.try_recv() {
            Ok(rendered) => Some(rendered),
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        self.pending_icons = None;
        let Some(window) = self.window(WindowId::PRIMARY) else {
            return;
        };
        match rendered {
            Some(rendered) => rendered.apply(window.as_ref()),
            None => apply_scene_window_icon(window.as_ref(), self.settings.icon.as_ref(), true),
        }
    }

    /// Counts a painter created for the startup (the startup thread's, or one
    /// built on the event thread while the startup is measured).
    pub(super) fn note_startup_painter(&self) {
        if self.startup.measuring() {
            self.startup
                .handle
                .update(|status| status.work.painters_created += 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECONDARY: WindowId = WindowId(7);

    fn ready(policy: StartupTakeover) -> StartupCoordinator {
        let mut startup = StartupCoordinator::new(true);
        startup.ui_ready(policy, 0);
        startup
    }

    #[test]
    fn nothing_can_be_requested_before_the_program_exists() {
        let mut startup = StartupCoordinator::new(true);
        assert_eq!(startup.ticket(), None);
        assert_eq!(
            startup.request(StartupTicket { generation: 1 }, 0),
            Err(StartupError::StaleTicket)
        );
        assert!(startup.holds_presents(WindowId::PRIMARY));
        assert!(!startup.completes_with(WindowId::PRIMARY, 9));
    }

    #[test]
    fn immediate_takeover_waits_for_a_frame_flushed_after_ready() {
        let mut startup = StartupCoordinator::new(true);
        assert!(startup.ui_ready(StartupTakeover::Immediate, 3));
        assert_eq!(startup.phase(), StartupPhase::TakeoverRequested);
        assert!(!startup.holds_presents(WindowId::PRIMARY));
        // A frame flushed before the request shows older content.
        assert!(!startup.completes_with(WindowId::PRIMARY, 3));
        assert!(startup.completes_with(WindowId::PRIMARY, 4));
    }

    #[test]
    fn deferred_takeover_keeps_the_primary_window_behind_the_splash() {
        let mut startup = ready(StartupTakeover::Deferred);
        assert_eq!(startup.phase(), StartupPhase::UiReady);
        assert!(startup.holds_presents(WindowId::PRIMARY));
        assert!(!startup.holds_presents(SECONDARY));
        assert!(!startup.completes_with(WindowId::PRIMARY, 50));
        let ticket = startup.ticket().unwrap();
        startup.request(ticket, 50).unwrap();
        assert!(!startup.holds_presents(WindowId::PRIMARY));
        assert!(!startup.completes_with(WindowId::PRIMARY, 50));
        assert!(startup.completes_with(WindowId::PRIMARY, 51));
    }

    #[test]
    fn other_windows_never_complete_the_takeover() {
        let startup = ready(StartupTakeover::Immediate);
        assert!(!startup.completes_with(SECONDARY, 10));
    }

    #[test]
    fn once_the_takeover_frame_is_committed_nothing_can_be_withdrawn() {
        let mut startup = ready(StartupTakeover::Immediate);
        let ticket = startup.ticket().unwrap();
        startup.frame_committed();
        assert_eq!(startup.cancel(ticket), Err(StartupError::AlreadyHandedOff));
        assert!(!startup.completes_with(WindowId::PRIMARY, 5));
        assert_eq!(startup.phase(), StartupPhase::TakeoverRequested);
    }

    #[test]
    fn a_cancel_retires_the_ticket_and_late_requests_are_refused() {
        let mut startup = ready(StartupTakeover::Deferred);
        let old = startup.ticket().unwrap();
        startup.request(old, 5).unwrap();
        startup.cancel(old).unwrap();
        assert_eq!(startup.phase(), StartupPhase::UiReady);
        assert!(startup.holds_presents(WindowId::PRIMARY));
        // A completion that was already in flight when the cancel happened.
        assert_eq!(startup.request(old, 9), Err(StartupError::StaleTicket));
        assert_eq!(startup.cancel(old), Err(StartupError::StaleTicket));
        assert!(!startup.completes_with(WindowId::PRIMARY, 9));
        let fresh = startup.ticket().unwrap();
        assert_ne!(fresh, old);
        startup.request(fresh, 9).unwrap();
        assert!(startup.completes_with(WindowId::PRIMARY, 10));
    }

    #[test]
    fn a_repeated_request_keeps_the_first_target() {
        let mut startup = ready(StartupTakeover::Deferred);
        let ticket = startup.ticket().unwrap();
        startup.request(ticket, 5).unwrap();
        startup.request(ticket, 8).unwrap();
        assert!(startup.completes_with(WindowId::PRIMARY, 6));
    }

    #[test]
    fn handed_off_is_final() {
        let mut startup = ready(StartupTakeover::Immediate);
        let ticket = startup.ticket().unwrap();
        startup.handed_off();
        assert_eq!(startup.ticket(), None);
        assert_eq!(
            startup.request(ticket, 99),
            Err(StartupError::AlreadyHandedOff)
        );
        assert_eq!(startup.cancel(ticket), Err(StartupError::AlreadyHandedOff));
        assert!(!startup.holds_presents(WindowId::PRIMARY));
        assert!(!startup.completes_with(WindowId::PRIMARY, 100));
    }

    #[test]
    fn without_a_splash_nothing_is_held_but_a_deferral_still_waits_for_its_request() {
        let mut startup = StartupCoordinator::new(false);
        assert!(!startup.holds_presents(WindowId::PRIMARY));
        startup.ui_ready(StartupTakeover::Deferred, 0);
        assert!(!startup.holds_presents(WindowId::PRIMARY));
        // Frames present, but the program has not asked: its ticket stays
        // valid, exactly as on a platform with a splash.
        assert!(!startup.completes_with(WindowId::PRIMARY, 1));
        let ticket = startup.ticket().expect("deferred program keeps its ticket");
        startup.request(ticket, 3).unwrap();
        assert!(startup.completes_with(WindowId::PRIMARY, 4));
    }

    #[test]
    fn ready_is_delivered_once() {
        let mut startup = StartupCoordinator::new(true);
        assert!(startup.ui_ready(StartupTakeover::Immediate, 0));
        assert!(!startup.ui_ready(StartupTakeover::Immediate, 4));
        assert!(startup.completes_with(WindowId::PRIMARY, 1));
    }
}
