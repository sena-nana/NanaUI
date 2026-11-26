//! Platform-neutral application and shell contract shared by the iOS and
//! Android hosts.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use nana_ui::runtime::FrameworkError;

use crate::render::MobileRenderer;
use crate::runtime::{MobileRuntime, TouchPhase};
use crate::schedule::FrameDemand;

/// Feedback the shell plays through the platform's haptic engine
/// (`UIFeedbackGenerator` on iOS, `View.performHapticFeedback` on Android).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Haptic {
    /// A light impact: a button press.
    Light = 0,
    /// A medium impact.
    Medium = 1,
    /// A selection tick: a value stepping.
    Selection = 2,
    /// A success notification.
    Success = 3,
    /// A warning notification.
    Warning = 4,
}

/// Whether the application's window is in the foreground: an app pauses work
/// that only the visible window needs (a camera, a discovery socket) while it
/// is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Resumed,
    Paused,
}

/// Platform-specific shell capabilities behind [`MobileShell`].
///
/// Every method may be called from any thread; implementations must be
/// thread-safe (dispatching UI work onto the platform's main thread when
/// required) and must not re-enter the host synchronously.
pub trait MobileShellBackend: Send + Sync + 'static {
    /// Ask the host loop / display link for a frame: state the document reads
    /// changed off the host thread.
    fn wake(&self);

    /// Play haptic feedback.
    fn haptic(&self, _style: Haptic) {}

    /// Keep the screen awake (`UIApplication.isIdleTimerDisabled` on iOS,
    /// `FLAG_KEEP_SCREEN_ON` on Android).
    fn set_keep_screen_awake(&self, _keep_awake: bool) {}

    /// Hide or show system chrome (status bar, home indicator / navigation
    /// bars).
    fn set_system_bars_hidden(&self, _hidden: bool) {}

    /// Open the operating system's Settings page for this application.
    fn open_app_settings(&self) {}

    /// The user's preferred language tags in priority order (BCP-47, e.g.
    /// `"zh-Hans"`, `"en-US"`).
    fn preferred_languages(&self) -> Vec<String> {
        Vec::new()
    }
}

struct NoopShellBackend;

impl MobileShellBackend for NoopShellBackend {
    fn wake(&self) {}
}

/// Thread-safe handle an application uses to reach the native mobile shell
/// from any thread.
#[derive(Clone)]
pub struct MobileShell {
    backend: Arc<dyn MobileShellBackend>,
}

impl fmt::Debug for MobileShell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MobileShell").finish_non_exhaustive()
    }
}

impl MobileShell {
    /// Wrap a platform shell backend.
    pub fn new(backend: impl MobileShellBackend) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }

    /// A shell whose methods are no-ops, useful in headless unit tests.
    pub fn noop() -> Self {
        Self::new(NoopShellBackend)
    }

    /// Ask the shell for a frame: state the document reads changed off the
    /// host thread.
    pub fn wake(&self) {
        self.backend.wake();
    }

    /// Play `style` through the platform's haptic engine.
    pub fn haptic(&self, style: Haptic) {
        self.backend.haptic(style);
    }

    /// Prevent or allow the display to dim and lock while the app is visible.
    pub fn set_keep_screen_awake(&self, keep_awake: bool) {
        self.backend.set_keep_screen_awake(keep_awake);
    }

    /// Hide or show platform system bars (status bar, home indicator /
    /// navigation bars).
    pub fn set_system_bars_hidden(&self, hidden: bool) {
        self.backend.set_system_bars_hidden(hidden);
    }

    /// Open the operating system's Settings screen for this application.
    pub fn open_app_settings(&self) {
        self.backend.open_app_settings();
    }

    /// Preferred system languages as BCP-47 tags in priority order.
    pub fn preferred_languages(&self) -> Vec<String> {
        self.backend.preferred_languages()
    }
}

/// The application a mobile host runs (shared by iOS and Android): it mounts
/// its view once, may prepare per-frame host state/textures, reports its own
/// frame deadlines, and receives foreground/background and non-consuming touch
/// activity notifications.
pub trait MobileApp: Any {
    fn mount(
        &mut self,
        runtime: &mut MobileRuntime,
        shell: MobileShell,
    ) -> Result<(), FrameworkError>;

    /// Runs on the host thread before each frame is prepared.
    fn before_frame(&mut self, _runtime: &mut MobileRuntime, _renderer: &mut MobileRenderer) {}

    /// A frame the app needs besides the document's (for example a periodic
    /// host texture upload or an app-owned timer deadline).
    fn demand(&self) -> FrameDemand {
        FrameDemand::Idle
    }

    /// Foreground / background transition.
    fn lifecycle(&mut self, _runtime: &mut MobileRuntime, _event: Lifecycle) {}

    /// Non-consuming touch activity notification fired whenever the shell
    /// observes a touch, regardless of widget hit-testing or consumption (for
    /// example idle-dim / auto-hide timers).
    fn on_touch_activity(&mut self, _runtime: &mut MobileRuntime, _phase: TouchPhase) {}
}
