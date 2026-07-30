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
    PrepareWindow,
    SyncMaximized,
    MaximizedChanged(bool),
}

/// Interaction state shared by title bar views and their window runtime.
#[derive(Debug, Clone)]
pub struct WindowChromeState {
    chrome: WindowChrome,
    cursor_position: Option<Point>,
    drag_origin: Option<Point>,
    maximized: bool,
}

impl WindowChromeState {
    pub fn new(chrome: WindowChrome) -> Self {
        Self {
            chrome,
            cursor_position: None,
            drag_origin: None,
            maximized: false,
        }
    }

    pub const fn chrome(&self) -> WindowChrome {
        self.chrome
    }

    pub const fn is_maximized(&self) -> bool {
        self.maximized
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
            WindowChromeEvent::MaximizedChanged(maximized) => {
                self.maximized = maximized;
                None
            }
            WindowChromeEvent::PrepareWindow => None,
            WindowChromeEvent::SyncMaximized => None,
        }
    }

    /// Handles an event using the standard Iced single-window runtime.
    pub fn update_iced(&mut self, event: WindowChromeEvent) -> Task<WindowChromeEvent> {
        if event == WindowChromeEvent::PrepareWindow {
            return prepare_window();
        }
        if event == WindowChromeEvent::SyncMaximized {
            return sync_maximized();
        }

        self.update(event)
            .map_or_else(Task::none, perform_window_action)
    }

    /// Tracks window creation and resize so the maximize/restore icon stays current.
    pub fn subscription() -> Subscription<WindowChromeEvent> {
        iced::event::listen_with(window_event)
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

fn perform_window_action(action: WindowChromeAction) -> Task<WindowChromeEvent> {
    window::latest().and_then(move |id| match action {
        WindowChromeAction::Drag => perform_drag(id),
        WindowChromeAction::Minimize => window::minimize::<WindowChromeEvent>(id, true),
        WindowChromeAction::ToggleMaximize => window::toggle_maximize::<WindowChromeEvent>(id)
            .chain(window::is_maximized(id).map(WindowChromeEvent::MaximizedChanged)),
        WindowChromeAction::Close => window::close::<WindowChromeEvent>(id),
    })
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

fn sync_maximized() -> Task<WindowChromeEvent> {
    window::latest()
        .and_then(|id| window::is_maximized(id).map(WindowChromeEvent::MaximizedChanged))
}

fn prepare_window() -> Task<WindowChromeEvent> {
    window::latest().and_then(|id| {
        window::run(id, |window| {
            let _ = nana_window::prepare_custom_title_bar(window);
            WindowChromeEvent::SyncMaximized
        })
    })
}

fn window_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: window::Id,
) -> Option<WindowChromeEvent> {
    match event {
        iced::Event::Window(window::Event::Opened { .. }) => Some(WindowChromeEvent::PrepareWindow),
        iced::Event::Window(window::Event::Resized(_)) => Some(WindowChromeEvent::SyncMaximized),
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
    use iced::Point;

    use super::{
        WindowChrome, WindowChromeAction, WindowChromeEvent, WindowChromeState, WindowControlMode,
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

        state.update(WindowChromeEvent::MaximizedChanged(false));
        assert!(!state.is_maximized());
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
