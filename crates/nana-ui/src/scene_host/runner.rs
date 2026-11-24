//! Desktop winit event-loop runner (`SceneRunner`) and `ApplicationHandler` loop.

use super::*;
use nana_diagnostics::stall::Callback;

pub(super) enum SceneRunner<Program: RuntimeProgram> {
    Loading(Box<startup::LoadingStartup<Program::Message>>),
    /// The window is up (with its splash, if any) and the device is being
    /// requested off the event thread.
    Starting(Box<startup::PendingStartup<Program::Message>>),
    Ready(Box<WindowManager<Program>>),
    Finished {
        startup_failure: Arc<Mutex<Option<String>>>,
    },
}

impl<Program: RuntimeProgram> SceneRunner<Program> {
    fn fail(&mut self, event_loop: &dyn ActiveEventLoop, message: impl Into<String>) {
        let slot = match self {
            Self::Loading(loading) => Arc::clone(&loading.startup_failure),
            Self::Starting(pending) => Arc::clone(pending.startup_failure()),
            Self::Finished { startup_failure } => Arc::clone(startup_failure),
            Self::Ready(ready) => Arc::clone(&ready.startup_failure),
        };
        let message = message.into();
        nana_diagnostics::fault!(
            nana_diagnostics::framework::host::STARTUP_FAILED;
            "{message}"
        );
        if let Ok(mut guard) = slot.lock() {
            *guard = Some(message);
        }
        // Dropping the pending startup releases its splash, window and
        // surface on this thread.
        *self = Self::Finished {
            startup_failure: slot,
        };
        event_loop.exit();
    }

    /// Takes the device the startup thread produced, if it has, and moves on:
    /// to `Ready`, to the next presentation target, or to a startup failure.
    fn poll_startup(&mut self, event_loop: &dyn ActiveEventLoop) {
        let Self::Starting(pending) = self else {
            return;
        };
        pending.poll_splash();
        let Some(result) = pending.take_device() else {
            return;
        };
        let slot = Arc::clone(pending.startup_failure());
        let Self::Starting(pending) = std::mem::replace(
            self,
            Self::Finished {
                startup_failure: Arc::clone(&slot),
            },
        ) else {
            unreachable!("checked Starting");
        };
        match pending.finish::<Program>(event_loop, result) {
            Ok(startup::StartupStep::Ready(ready)) => *self = Self::Ready(ready),
            Ok(startup::StartupStep::Retry(pending)) => *self = Self::Starting(pending),
            Err(error) => self.fail(event_loop, error),
        }
    }

    /// When the startup times event-loop callbacks, the start of this one.
    fn block_started(&self) -> Option<Instant> {
        let measures = match self {
            Self::Starting(_) => true,
            Self::Ready(ready) => ready.startup.measures_blocks(),
            Self::Loading(_) | Self::Finished { .. } => false,
        };
        measures.then(Instant::now)
    }

    /// Longest event-thread callback while the startup is measured.
    fn note_block(&mut self, started: Option<Instant>) {
        let Some(started) = started else {
            return;
        };
        match self {
            Self::Starting(pending) => pending.note_block(started.elapsed()),
            Self::Ready(ready) => ready.note_startup_block(started.elapsed()),
            Self::Loading(_) | Self::Finished { .. } => {}
        }
    }
}

impl<Program: RuntimeProgram> ApplicationHandler for SceneRunner<Program> {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !matches!(self, Self::Loading(_)) {
            return;
        }
        let _busy = nana_diagnostics::stall::busy(Callback::Resumed);
        let started = Some(Instant::now());
        let Self::Loading(loading) = std::mem::replace(
            self,
            Self::Finished {
                startup_failure: Arc::new(Mutex::new(None)),
            },
        ) else {
            unreachable!("checked Loading");
        };
        let slot = Arc::clone(&loading.startup_failure);
        match startup::PendingStartup::begin::<Program>(event_loop, *loading) {
            Ok(pending) => *self = Self::Starting(Box::new(pending)),
            Err(error) => {
                *self = Self::Finished {
                    startup_failure: slot,
                };
                self.fail(event_loop, error);
            }
        }
        self.note_block(started);
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        let _busy = nana_diagnostics::stall::busy(Callback::WakeUp);
        let started = self.block_started();
        match self {
            Self::Starting(_) => self.poll_startup(event_loop),
            Self::Ready(ready) => ready.proxy_wake_up(event_loop),
            Self::Loading(_) | Self::Finished { .. } => {}
        }
        self.note_block(started);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WinitWindowEvent,
    ) {
        let _busy = nana_diagnostics::stall::busy(Callback::WindowEvent);
        let started = self.block_started();
        match self {
            Self::Starting(pending) => {
                if let WinitWindowEvent::ScaleFactorChanged { scale_factor, .. } = &event {
                    pending.rescale_splash(window_id, *scale_factor);
                }
                // The program does not exist yet; closing the window cancels
                // the startup. Nothing was initialized, so nothing is
                // reported as a failure.
                if pending.owns(window_id) && matches!(event, WinitWindowEvent::CloseRequested) {
                    let slot = Arc::clone(pending.startup_failure());
                    if let Self::Starting(pending) = std::mem::replace(
                        self,
                        Self::Finished {
                            startup_failure: slot,
                        },
                    ) {
                        pending.cancel();
                    }
                    event_loop.exit();
                    return;
                }
            }
            Self::Ready(ready) => {
                let Some(id) = ready.window_ids.get(&window_id).copied() else {
                    return;
                };
                ready.handle_window_event(event_loop, id, event);
            }
            Self::Loading(_) | Self::Finished { .. } => return,
        }
        self.note_block(started);
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        let _busy = nana_diagnostics::stall::busy(Callback::AboutToWait);
        let started = self.block_started();
        let Self::Ready(ready) = self else {
            return;
        };
        ready.about_to_wait(event_loop);
        self.note_block(started);
    }
}
