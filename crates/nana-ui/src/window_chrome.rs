use iced::{Point, Subscription, Task, window};

const DRAG_THRESHOLD: f32 = 4.0;
#[cfg(target_os = "macos")]
const MACOS_TRAFFIC_LIGHT_INSET: f32 = 78.0;

/// Selects whether window controls are supplied by the platform or NanaUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowControlMode {
    NativeLeading,
    NativeTrailing,
    Custom,
}

/// Platform presentation contract for a NanaUI application title bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowChrome {
    pub controls: WindowControlMode,
    pub leading_inset: f32,
    pub trailing_inset: f32,
}

impl WindowChrome {
    pub fn new(controls: WindowControlMode, leading_inset: f32, trailing_inset: f32) -> Self {
        Self {
            controls,
            leading_inset: valid_inset(leading_inset),
            trailing_inset: valid_inset(trailing_inset),
        }
    }

    pub const fn custom() -> Self {
        Self {
            controls: WindowControlMode::Custom,
            leading_inset: 0.0,
            trailing_inset: 0.0,
        }
    }

    pub fn native_leading(leading_inset: f32) -> Self {
        Self {
            controls: WindowControlMode::NativeLeading,
            leading_inset: valid_inset(leading_inset),
            trailing_inset: 0.0,
        }
    }

    pub fn native_trailing(trailing_inset: f32) -> Self {
        Self {
            controls: WindowControlMode::NativeTrailing,
            leading_inset: 0.0,
            trailing_inset: valid_inset(trailing_inset),
        }
    }

    pub const fn uses_custom_controls(self) -> bool {
        matches!(self.controls, WindowControlMode::Custom)
    }

    pub fn platform_default() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::native_leading(MACOS_TRAFFIC_LIGHT_INSET)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Self::custom()
        }
    }
}

impl Default for WindowChrome {
    fn default() -> Self {
        Self::platform_default()
    }
}

/// A real operation requested by the custom title bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowChromeAction {
    Drag,
    Minimize,
    ToggleMaximize,
    Close,
}

/// Input handled by [`WindowChromeState`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowChromeEvent {
    PointerMoved(Point),
    PointerPressed,
    PointerReleased,
    PointerCancelled,
    Action(WindowChromeAction),
    PrepareWindow(window::Id),
    SyncMaximized(window::Id),
    WindowClosed(window::Id),
    MaximizedChanged { window: window::Id, maximized: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IcedWindowCommand {
    Prepare(window::Id),
    SyncMaximized(window::Id),
    Action {
        window: window::Id,
        action: WindowChromeAction,
    },
}

/// Interaction state shared by title bar views and their window runtime.
#[derive(Debug, Clone)]
pub struct WindowChromeState {
    chrome: WindowChrome,
    window: Option<window::Id>,
    auto_bind: bool,
    cursor_position: Option<Point>,
    drag_origin: Option<Point>,
    maximized: bool,
}

impl WindowChromeState {
    /// Creates a state that binds to the first window opened by the Iced runtime.
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

    /// Creates a state bound to a specific Iced window.
    pub fn for_window(window: window::Id, chrome: WindowChrome) -> Self {
        let mut state = Self::new(chrome);
        state.bind(window);
        state
    }

    /// Binds this state to a specific Iced window.
    ///
    /// Binding explicitly disables automatic rebinding after the window closes.
    pub fn bind(&mut self, window: window::Id) {
        self.auto_bind = false;
        self.set_window(Some(window));
    }

    pub const fn window_id(&self) -> Option<window::Id> {
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
            WindowChromeEvent::PrepareWindow(_) | WindowChromeEvent::SyncMaximized(_) => None,
        }
    }

    /// Handles an event using the standard Iced window runtime.
    pub fn update_iced(&mut self, event: WindowChromeEvent) -> Task<WindowChromeEvent> {
        match self.iced_command(event) {
            Some(IcedWindowCommand::Prepare(window)) => prepare_window(window),
            Some(IcedWindowCommand::SyncMaximized(window)) => sync_maximized(window),
            Some(IcedWindowCommand::Action { window, action }) => {
                perform_window_action(window, action)
            }
            None => Task::none(),
        }
    }

    /// Tracks window creation, resize, and close events for all application windows.
    ///
    /// Each [`WindowChromeState`] filters this stream against its own binding.
    pub fn subscription() -> Subscription<WindowChromeEvent> {
        iced::event::listen_with(window_event)
    }

    fn iced_command(&mut self, event: WindowChromeEvent) -> Option<IcedWindowCommand> {
        match event {
            WindowChromeEvent::PrepareWindow(window) => {
                if self.window.is_none() && self.auto_bind {
                    self.set_window(Some(window));
                }
                (self.window == Some(window)).then_some(IcedWindowCommand::Prepare(window))
            }
            WindowChromeEvent::SyncMaximized(window) => {
                (self.window == Some(window)).then_some(IcedWindowCommand::SyncMaximized(window))
            }
            WindowChromeEvent::WindowClosed(window) => {
                self.update(WindowChromeEvent::WindowClosed(window));
                None
            }
            WindowChromeEvent::MaximizedChanged { window, maximized } => {
                self.update(WindowChromeEvent::MaximizedChanged { window, maximized });
                None
            }
            event => {
                let window = self.window?;
                self.update(event)
                    .map(|action| IcedWindowCommand::Action { window, action })
            }
        }
    }

    fn set_window(&mut self, window: Option<window::Id>) {
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

/// Applies NanaUI's custom-title-bar attributes to a standard Iced window.
pub fn custom_title_bar_window(mut settings: window::Settings) -> window::Settings {
    #[cfg(target_os = "macos")]
    {
        settings.decorations = true;
        settings.platform_specific.title_hidden = true;
        settings.platform_specific.titlebar_transparent = true;
        settings.platform_specific.fullsize_content_view = true;
    }

    #[cfg(not(target_os = "macos"))]
    {
        settings.decorations = false;
    }

    settings
}

fn perform_window_action(
    window: window::Id,
    action: WindowChromeAction,
) -> Task<WindowChromeEvent> {
    match action {
        WindowChromeAction::Drag => perform_drag(window),
        WindowChromeAction::Minimize => window::minimize::<WindowChromeEvent>(window, true),
        WindowChromeAction::ToggleMaximize => window::toggle_maximize::<WindowChromeEvent>(window)
            .chain(
                window::is_maximized(window).map(move |maximized| {
                    WindowChromeEvent::MaximizedChanged { window, maximized }
                }),
            ),
        WindowChromeAction::Close => window::close::<WindowChromeEvent>(window),
    }
}

#[cfg(target_os = "macos")]
fn perform_drag(id: window::Id) -> Task<WindowChromeEvent> {
    window::run(id, |window| {
        let _ = nana_window::drag_custom_title_bar(window);
    })
    .discard()
}

#[cfg(not(target_os = "macos"))]
fn perform_drag(id: window::Id) -> Task<WindowChromeEvent> {
    window::drag(id)
}

fn sync_maximized(window: window::Id) -> Task<WindowChromeEvent> {
    window::is_maximized(window)
        .map(move |maximized| WindowChromeEvent::MaximizedChanged { window, maximized })
}

fn prepare_window(window: window::Id) -> Task<WindowChromeEvent> {
    window::run(window, move |handle| {
        let _ = nana_window::prepare_custom_title_bar(handle);
        WindowChromeEvent::SyncMaximized(window)
    })
}

fn window_event(
    event: iced::Event,
    _status: iced::event::Status,
    window: window::Id,
) -> Option<WindowChromeEvent> {
    match event {
        iced::Event::Window(window::Event::Opened { .. }) => {
            Some(WindowChromeEvent::PrepareWindow(window))
        }
        iced::Event::Window(window::Event::Resized(_)) => {
            Some(WindowChromeEvent::SyncMaximized(window))
        }
        iced::Event::Window(window::Event::Closed) => Some(WindowChromeEvent::WindowClosed(window)),
        _ => None,
    }
}

fn valid_inset(value: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

fn distance(from: Point, to: Point) -> f32 {
    (to.x - from.x).hypot(to.y - from.y)
}

#[cfg(test)]
mod tests {
    use iced::{Point, Size, event, window};

    use super::{
        IcedWindowCommand, WindowChrome, WindowChromeAction, WindowChromeEvent, WindowChromeState,
        WindowControlMode, window_event,
    };

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn drag_starts_once_only_after_crossing_the_threshold() {
        let mut state = WindowChromeState::default();

        assert_eq!(
            state.update(WindowChromeEvent::PointerMoved(Point::new(10.0, 10.0))),
            None
        );
        assert_eq!(state.update(WindowChromeEvent::PointerPressed), None);
        assert_eq!(
            state.update(WindowChromeEvent::PointerMoved(Point::new(12.0, 11.0))),
            None
        );
        assert_eq!(
            state.update(WindowChromeEvent::PointerMoved(Point::new(15.0, 10.0))),
            Some(WindowChromeAction::Drag)
        );
        assert_eq!(
            state.update(WindowChromeEvent::PointerMoved(Point::new(20.0, 10.0))),
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
            state.update(WindowChromeEvent::PointerMoved(Point::new(4.0, 4.0)));
            state.update(WindowChromeEvent::PointerPressed);
            state.update(end);

            assert_eq!(
                state.update(WindowChromeEvent::PointerMoved(Point::new(20.0, 20.0))),
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
    fn window_events_preserve_their_source_window() {
        let window = window::Id::unique();
        let opened = iced::Event::Window(window::Event::Opened {
            position: None,
            size: Size::new(800.0, 600.0),
            scale_factor: 1.0,
        });
        let resized = iced::Event::Window(window::Event::Resized(Size::new(900.0, 700.0)));
        let closed = iced::Event::Window(window::Event::Closed);

        assert_eq!(
            window_event(opened, event::Status::Ignored, window),
            Some(WindowChromeEvent::PrepareWindow(window))
        );
        assert_eq!(
            window_event(resized, event::Status::Ignored, window),
            Some(WindowChromeEvent::SyncMaximized(window))
        );
        assert_eq!(
            window_event(closed, event::Status::Ignored, window),
            Some(WindowChromeEvent::WindowClosed(window))
        );
    }

    #[test]
    fn explicit_states_route_rebind_and_ignore_stale_results() {
        let window_a = window::Id::unique();
        let window_b = window::Id::unique();
        let mut state_a = WindowChromeState::for_window(window_a, WindowChrome::platform_default());
        let mut state_b = WindowChromeState::for_window(window_b, WindowChrome::platform_default());

        assert_eq!(
            state_a.iced_command(WindowChromeEvent::SyncMaximized(window_b)),
            None
        );
        assert_eq!(
            state_a.iced_command(WindowChromeEvent::SyncMaximized(window_a)),
            Some(IcedWindowCommand::SyncMaximized(window_a))
        );
        assert_eq!(
            state_a.iced_command(WindowChromeEvent::Action(WindowChromeAction::Minimize)),
            Some(IcedWindowCommand::Action {
                window: window_a,
                action: WindowChromeAction::Minimize,
            })
        );
        assert_eq!(
            state_b.iced_command(WindowChromeEvent::Action(WindowChromeAction::Close)),
            Some(IcedWindowCommand::Action {
                window: window_b,
                action: WindowChromeAction::Close,
            })
        );

        state_a.iced_command(WindowChromeEvent::WindowClosed(window_a));
        assert_eq!(state_a.window_id(), None);
        assert_eq!(
            state_a.iced_command(WindowChromeEvent::PrepareWindow(window_b)),
            None
        );

        state_a.bind(window_b);
        state_a.iced_command(WindowChromeEvent::MaximizedChanged {
            window: window_a,
            maximized: true,
        });
        assert!(!state_a.is_maximized());
        assert_eq!(
            state_a.iced_command(WindowChromeEvent::Action(
                WindowChromeAction::ToggleMaximize
            )),
            Some(IcedWindowCommand::Action {
                window: window_b,
                action: WindowChromeAction::ToggleMaximize,
            })
        );
        assert!(state_a.is_maximized());
    }

    #[test]
    fn automatic_state_binds_first_window_and_rebinds_after_it_closes() {
        let window_a = window::Id::unique();
        let window_b = window::Id::unique();
        let mut state = WindowChromeState::default();

        assert_eq!(state.window_id(), None);
        assert_eq!(
            state.iced_command(WindowChromeEvent::PrepareWindow(window_a)),
            Some(IcedWindowCommand::Prepare(window_a))
        );
        assert_eq!(state.window_id(), Some(window_a));
        assert_eq!(
            state.iced_command(WindowChromeEvent::PrepareWindow(window_b)),
            None
        );

        assert_eq!(
            state.iced_command(WindowChromeEvent::WindowClosed(window_a)),
            None
        );
        assert_eq!(state.window_id(), None);
        assert_eq!(
            state.iced_command(WindowChromeEvent::PrepareWindow(window_b)),
            Some(IcedWindowCommand::Prepare(window_b))
        );
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
    fn iced_window_settings_use_the_platform_titlebar_contract() {
        let settings = super::custom_title_bar_window(iced::window::Settings::default());

        #[cfg(target_os = "macos")]
        {
            assert!(settings.decorations);
            assert!(settings.platform_specific.title_hidden);
            assert!(settings.platform_specific.titlebar_transparent);
            assert!(settings.platform_specific.fullsize_content_view);
            assert_eq!(
                WindowChrome::platform_default().controls,
                WindowControlMode::NativeLeading
            );
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert!(!settings.decorations);
            assert_eq!(
                WindowChrome::platform_default().controls,
                WindowControlMode::Custom
            );
        }
    }
}
