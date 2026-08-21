use nana_ui_core::LogicalPoint;
use nana_ui_platform::WindowId;

pub use nana_ui_core::{WindowChrome, WindowChromeAction, WindowControlMode};

const DRAG_THRESHOLD: f32 = 4.0;

/// Input handled by [`WindowChromeState`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowChromeEvent {
    PointerMoved(LogicalPoint),
    PointerPressed,
    PointerReleased,
    PointerCancelled,
    Action(WindowChromeAction),
    PrepareWindow(WindowId),
    SyncMaximized(WindowId),
    WindowClosed(WindowId),
    MaximizedChanged { window: WindowId, maximized: bool },
}

/// Interaction state shared by title bar views and their window runtime.
#[derive(Debug, Clone)]
pub struct WindowChromeState {
    chrome: WindowChrome,
    window: Option<WindowId>,
    auto_bind: bool,
    cursor_position: Option<LogicalPoint>,
    drag_origin: Option<LogicalPoint>,
    maximized: bool,
}

impl WindowChromeState {
    /// Creates a state that binds to the first window opened by the host.
    pub fn new(chrome: WindowChrome) -> Self {
        Self {
            chrome,
            window: None,
            auto_bind: true,
            cursor_position: None,
            drag_origin: None,
            maximized: false,
        }
    }

    /// Creates a state bound to a specific host window.
    pub fn for_window(window: WindowId, chrome: WindowChrome) -> Self {
        let mut state = Self::new(chrome);
        state.bind(window);
        state
    }

    /// Binds this state to a specific host window.
    ///
    /// Binding explicitly disables automatic rebinding after the window closes.
    pub fn bind(&mut self, window: WindowId) {
        self.auto_bind = false;
        self.set_window(Some(window));
    }

    pub const fn window_id(&self) -> Option<WindowId> {
        self.window
    }

    pub const fn chrome(&self) -> WindowChrome {
        self.chrome
    }

    pub const fn is_maximized(&self) -> bool {
        self.maximized
    }

    /// Synchronizes the visible maximize/restore state for a host-owned window.
    pub fn set_maximized(&mut self, maximized: bool) {
        self.maximized = maximized;
    }

    /// Reduces a title bar event and returns a runtime action when one is ready.
    pub fn update(&mut self, event: WindowChromeEvent) -> Option<WindowChromeAction> {
        match event {
            WindowChromeEvent::PointerMoved(position) => {
                self.cursor_position = Some(position);
                let origin = self.drag_origin?;
                if distance(origin, position) < DRAG_THRESHOLD {
                    return None;
                }
                self.drag_origin = None;
                Some(WindowChromeAction::Drag)
            }
            WindowChromeEvent::PointerPressed => {
                #[cfg(target_os = "macos")]
                {
                    Some(WindowChromeAction::Drag)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.drag_origin = self.cursor_position;
                    None
                }
            }
            WindowChromeEvent::PointerReleased | WindowChromeEvent::PointerCancelled => {
                self.drag_origin = None;
                None
            }
            WindowChromeEvent::Action(action) => {
                if action == WindowChromeAction::ToggleMaximize {
                    self.maximized = !self.maximized;
                }
                Some(action)
            }
            WindowChromeEvent::MaximizedChanged { window, maximized } => {
                if self.window == Some(window) {
                    self.set_maximized(maximized);
                }
                None
            }
            WindowChromeEvent::WindowClosed(window) => {
                if self.window == Some(window) {
                    self.set_window(None);
                }
                None
            }
            WindowChromeEvent::PrepareWindow(window) => {
                if self.window.is_none() && self.auto_bind {
                    self.set_window(Some(window));
                }
                None
            }
            WindowChromeEvent::SyncMaximized(_) => None,
        }
    }

    fn set_window(&mut self, window: Option<WindowId>) {
        self.window = window;
        self.cursor_position = None;
        self.drag_origin = None;
        self.maximized = false;
    }
}

impl Default for WindowChromeState {
    fn default() -> Self {
        Self::new(WindowChrome::platform_default())
    }
}

fn distance(from: LogicalPoint, to: LogicalPoint) -> f32 {
    (to.x - from.x).hypot(to.y - from.y)
}

#[cfg(test)]
mod tests {
    use super::{
        WindowChrome, WindowChromeAction, WindowChromeEvent, WindowChromeState, WindowControlMode,
    };
    use nana_ui_core::LogicalPoint;
    use nana_ui_platform::WindowId;

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn drag_starts_once_only_after_crossing_the_threshold() {
        let mut state = WindowChromeState::default();

        assert_eq!(
            state.update(WindowChromeEvent::PointerMoved(LogicalPoint::new(
                10.0, 10.0
            ))),
            None
        );
        assert_eq!(state.update(WindowChromeEvent::PointerPressed), None);
        assert_eq!(
            state.update(WindowChromeEvent::PointerMoved(LogicalPoint::new(
                12.0, 11.0
            ))),
            None
        );
        assert_eq!(
            state.update(WindowChromeEvent::PointerMoved(LogicalPoint::new(
                15.0, 10.0
            ))),
            Some(WindowChromeAction::Drag)
        );
        assert_eq!(
            state.update(WindowChromeEvent::PointerMoved(LogicalPoint::new(
                20.0, 10.0
            ))),
            None
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_delegates_drag_threshold_to_appkit_on_blank_press() {
        let mut state = WindowChromeState::default();

        assert_eq!(
            state.update(WindowChromeEvent::PointerPressed),
            Some(WindowChromeAction::Drag)
        );
    }

    #[test]
    fn release_and_cancel_clear_a_pending_drag() {
        for end in [
            WindowChromeEvent::PointerReleased,
            WindowChromeEvent::PointerCancelled,
        ] {
            let mut state = WindowChromeState::default();
            state.update(WindowChromeEvent::PointerMoved(LogicalPoint::new(4.0, 4.0)));
            state.update(WindowChromeEvent::PointerPressed);
            state.update(end);

            assert_eq!(
                state.update(WindowChromeEvent::PointerMoved(LogicalPoint::new(
                    20.0, 20.0
                ))),
                None
            );
        }
    }

    #[test]
    fn maximize_action_and_runtime_sync_update_visible_state() {
        let mut state = WindowChromeState::default();

        assert_eq!(
            state.update(WindowChromeEvent::Action(
                WindowChromeAction::ToggleMaximize
            )),
            Some(WindowChromeAction::ToggleMaximize)
        );
        assert!(state.is_maximized());

        state.set_maximized(false);
        assert!(!state.is_maximized());
    }

    #[test]
    fn explicit_states_bind_and_ignore_stale_windows() {
        let window_a = WindowId(1);
        let window_b = WindowId(2);
        let mut state_a = WindowChromeState::for_window(window_a, WindowChrome::platform_default());
        let mut state_b = WindowChromeState::for_window(window_b, WindowChrome::platform_default());

        assert_eq!(
            state_a.update(WindowChromeEvent::Action(WindowChromeAction::Minimize)),
            Some(WindowChromeAction::Minimize)
        );
        assert_eq!(
            state_b.update(WindowChromeEvent::Action(WindowChromeAction::Close)),
            Some(WindowChromeAction::Close)
        );

        state_a.update(WindowChromeEvent::WindowClosed(window_a));
        assert_eq!(state_a.window_id(), None);
        state_a.update(WindowChromeEvent::PrepareWindow(window_b));
        assert_eq!(state_a.window_id(), None);

        state_a.bind(window_b);
        state_a.update(WindowChromeEvent::MaximizedChanged {
            window: window_a,
            maximized: true,
        });
        assert!(!state_a.is_maximized());
        assert_eq!(
            state_a.update(WindowChromeEvent::Action(
                WindowChromeAction::ToggleMaximize
            )),
            Some(WindowChromeAction::ToggleMaximize)
        );
        assert!(state_a.is_maximized());
    }

    #[test]
    fn automatic_state_binds_first_window_and_rebinds_after_it_closes() {
        let window_a = WindowId(1);
        let window_b = WindowId(2);
        let mut state = WindowChromeState::default();

        assert_eq!(state.window_id(), None);
        state.update(WindowChromeEvent::PrepareWindow(window_a));
        assert_eq!(state.window_id(), Some(window_a));
        state.update(WindowChromeEvent::PrepareWindow(window_b));
        assert_eq!(state.window_id(), Some(window_a));

        state.update(WindowChromeEvent::WindowClosed(window_a));
        assert_eq!(state.window_id(), None);
        state.update(WindowChromeEvent::PrepareWindow(window_b));
        assert_eq!(state.window_id(), Some(window_b));
    }

    #[test]
    fn control_actions_are_forwarded_without_rewriting_their_meaning() {
        let mut state = WindowChromeState::default();

        for action in [
            WindowChromeAction::Drag,
            WindowChromeAction::Minimize,
            WindowChromeAction::Close,
        ] {
            assert_eq!(
                state.update(WindowChromeEvent::Action(action)),
                Some(action)
            );
        }
    }

    #[test]
    fn chrome_normalizes_insets_and_exposes_control_mode() {
        let chrome = WindowChrome::new(WindowControlMode::NativeTrailing, f32::NAN, -10.0);
        assert_eq!(chrome.leading_inset, 0.0);
        assert_eq!(chrome.trailing_inset, 0.0);
        assert!(!chrome.uses_custom_controls());
        assert!(WindowChrome::custom().uses_custom_controls());
    }

    #[test]
    fn native_control_hit_excludes_leading_traffic_light_inset() {
        use nana_ui_core::LogicalRect;
        let chrome = WindowChrome::native_leading(78.0);
        let bar = LogicalRect::new(0.0, 0.0, 800.0, 36.0);
        assert!(chrome.native_control_hit(bar, 12.0, 18.0));
        assert!(!chrome.native_control_hit(bar, 90.0, 18.0));
        assert!(!chrome.native_control_hit(bar, 12.0, 40.0));
        assert!(!WindowChrome::custom().native_control_hit(bar, 12.0, 18.0));
    }
}
