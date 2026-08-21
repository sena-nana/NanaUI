use nana_ui_core::{LengthSpec, LogicalPoint};
use nana_ui_platform::{InputEvent, PointerPhase, WindowCommand, WindowId};
use nana_ui_runtime::{
    AccessibilityRole, AppContext, AppTitleBar, AppTitleBarControls, DocumentId, Entity, NodeKind,
};

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

/// Tracks a title-bar pointer gesture and emits [`WindowChromeEvent`]s.
#[derive(Debug, Clone, Default)]
pub struct TitleBarDragTracker {
    pressed: bool,
    control: Option<WindowChromeAction>,
}

impl TitleBarDragTracker {
    pub fn events(
        &mut self,
        context: &AppContext,
        document: DocumentId,
        event: &InputEvent,
    ) -> Vec<WindowChromeEvent> {
        match event {
            InputEvent::Pointer {
                phase: PointerPhase::Down,
                button: 0,
                is_primary: true,
                x,
                y,
                ..
            } => match title_bar_pointer_hit(context, document, *x, *y) {
                TitleBarHit::None => Vec::new(),
                TitleBarHit::Control(action) => {
                    self.pressed = true;
                    self.control = Some(action);
                    Vec::new()
                }
                TitleBarHit::Drag => {
                    self.pressed = true;
                    self.control = None;
                    vec![
                        WindowChromeEvent::PointerMoved(LogicalPoint::new(*x, *y)),
                        WindowChromeEvent::PointerPressed,
                    ]
                }
            },
            InputEvent::Pointer {
                phase: PointerPhase::Move,
                x,
                y,
                ..
            } if self.pressed && self.control.is_none() => {
                vec![WindowChromeEvent::PointerMoved(LogicalPoint::new(*x, *y))]
            }
            InputEvent::Pointer {
                phase: PointerPhase::Up,
                button: 0,
                x,
                y,
                ..
            } if self.pressed => {
                self.pressed = false;
                if let Some(action) = self.control.take() {
                    if title_bar_pointer_hit(context, document, *x, *y)
                        == TitleBarHit::Control(action)
                    {
                        vec![WindowChromeEvent::Action(action)]
                    } else {
                        Vec::new()
                    }
                } else {
                    vec![WindowChromeEvent::PointerReleased]
                }
            }
            InputEvent::Pointer {
                phase: PointerPhase::Cancel,
                ..
            } if self.pressed => {
                self.pressed = false;
                self.control = None;
                vec![WindowChromeEvent::PointerCancelled]
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TitleBarHit {
    None,
    Drag,
    Control(WindowChromeAction),
}

/// True when `(x, y)` is blank custom-title-bar chrome (not traffic lights or buttons).
#[cfg(test)]
fn title_bar_drag_hit(context: &AppContext, document: DocumentId, x: f32, y: f32) -> bool {
    matches!(
        title_bar_pointer_hit(context, document, x, y),
        TitleBarHit::Drag
    )
}

fn title_bar_pointer_hit(
    context: &AppContext,
    document: DocumentId,
    x: f32,
    y: f32,
) -> TitleBarHit {
    let mut current = context.pointer_target(document, x, y);
    let mut control = None;
    while let Some(id) = current {
        if control.is_none() {
            control = window_control_action(context, id);
        }
        if is_app_title_bar(context, id) {
            if let Some(action) = control {
                return TitleBarHit::Control(action);
            }
            if native_title_bar_control_hit(context, id, x, y) {
                return TitleBarHit::None;
            }
            return TitleBarHit::Drag;
        }
        if is_title_bar_control(context, id) && control.is_none() {
            return TitleBarHit::None;
        }
        current = context.world().node(id).and_then(|node| node.parent);
    }
    TitleBarHit::None
}

/// Maps a custom-title-bar chrome action to host window commands.
pub fn window_commands_for_chrome_action(
    id: WindowId,
    action: WindowChromeAction,
    maximized: bool,
) -> Vec<WindowCommand> {
    match action {
        WindowChromeAction::Drag => vec![WindowCommand::Drag(id)],
        WindowChromeAction::Minimize => vec![WindowCommand::SetMinimized {
            id,
            minimized: true,
        }],
        WindowChromeAction::ToggleMaximize => {
            vec![WindowCommand::SetMaximized { id, maximized }]
        }
        WindowChromeAction::Close => vec![WindowCommand::Close(id)],
    }
}

/// Reduces title-bar pointer input through [`TitleBarDragTracker`] and [`WindowChromeState`].
pub fn apply_title_bar_pointer(
    state: &mut WindowChromeState,
    tracker: &mut TitleBarDragTracker,
    context: &AppContext,
    document: DocumentId,
    event: &InputEvent,
) -> Option<WindowChromeAction> {
    let mut action = None;
    for chrome_event in tracker.events(context, document, event) {
        action = state.update(chrome_event).or(action);
    }
    action
}

fn is_app_title_bar(context: &AppContext, id: nana_ui_runtime::StableNodeId) -> bool {
    context
        .read(Entity::<AppTitleBar>::from_stable_id(id), |_| ())
        .is_ok()
        || context.world().node(id).is_some_and(|node| {
            matches!(
                &node.kind,
                NodeKind::Element { tag }
                    if tag == "app-title-bar" || tag == "nana-app-title-bar"
            )
        })
}

fn window_control_action(
    context: &AppContext,
    id: nana_ui_runtime::StableNodeId,
) -> Option<WindowChromeAction> {
    let parent = context.world().node(id).and_then(|node| node.parent)?;
    if !is_title_bar_controls(context, parent) {
        return None;
    }
    let children = context.world().node(parent)?.children;
    match children.iter().position(|child| *child == id) {
        Some(0) => Some(WindowChromeAction::Minimize),
        Some(1) => Some(WindowChromeAction::ToggleMaximize),
        Some(2) => Some(WindowChromeAction::Close),
        _ => None,
    }
}

fn is_title_bar_controls(context: &AppContext, id: nana_ui_runtime::StableNodeId) -> bool {
    context
        .read(Entity::<AppTitleBarControls>::from_stable_id(id), |_| ())
        .is_ok()
        || context.world().node(id).is_some_and(|node| {
            matches!(
                &node.kind,
                NodeKind::Element { tag } if tag == "app-title-bar-controls"
            )
        })
}

fn native_title_bar_control_hit(
    context: &AppContext,
    title_bar: nana_ui_runtime::StableNodeId,
    x: f32,
    y: f32,
) -> bool {
    let Some(bounds) = context.world().layout_box(title_bar) else {
        return false;
    };
    if let Ok(hit) = context.read(Entity::<AppTitleBar>::from_stable_id(title_bar), |bar| {
        bar.native_control_hit(bounds, x, y)
    }) {
        return hit;
    }
    let style = context.world().node_style(title_bar);
    let leading = style
        .map(|s| px_length(s.layout.padding_left))
        .unwrap_or(0.0);
    let trailing = style
        .map(|s| px_length(s.layout.padding_right))
        .unwrap_or(0.0);
    WindowChrome::new(
        if leading > 0.0 {
            WindowControlMode::NativeLeading
        } else if trailing > 0.0 {
            WindowControlMode::NativeTrailing
        } else {
            WindowControlMode::Custom
        },
        leading,
        trailing,
    )
    .native_control_hit(
        nana_ui_core::LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
        x,
        y,
    )
}

fn is_title_bar_control(context: &AppContext, id: nana_ui_runtime::StableNodeId) -> bool {
    context
        .world()
        .accessibility(id)
        .is_some_and(|state| state.role == AccessibilityRole::Button)
}

fn px_length(spec: Option<LengthSpec>) -> f32 {
    match spec {
        Some(LengthSpec::Px(value)) => value,
        _ => 0.0,
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

    fn pointer_down(x: f32, y: f32) -> nana_ui_platform::InputEvent {
        nana_ui_platform::InputEvent::Pointer {
            phase: nana_ui_platform::PointerPhase::Down,
            pointer_id: 1,
            pointer_type: nana_ui_platform::PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: 1,
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            modifiers: nana_ui_platform::InputModifiers::default(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn pointer_move(x: f32, y: f32) -> nana_ui_platform::InputEvent {
        let mut event = pointer_down(x, y);
        if let nana_ui_platform::InputEvent::Pointer { phase, buttons, .. } = &mut event {
            *phase = nana_ui_platform::PointerPhase::Move;
            *buttons = 1;
        }
        event
    }

    #[cfg(not(target_os = "macos"))]
    fn pointer_up(x: f32, y: f32) -> nana_ui_platform::InputEvent {
        let mut event = pointer_down(x, y);
        if let nana_ui_platform::InputEvent::Pointer { phase, buttons, .. } = &mut event {
            *phase = nana_ui_platform::PointerPhase::Up;
            *buttons = 0;
        }
        event
    }

    fn title_bar_document() -> (
        nana_ui_runtime::AppContext,
        nana_ui_runtime::DocumentId,
        nana_ui_runtime::Entity<nana_ui_runtime::AppTitleBar>,
        nana_ui_runtime::Entity<nana_ui_runtime::IconButton>,
    ) {
        use nana_ui_core::{ControlSize, Icon};
        use nana_ui_runtime::{AppContext, AppTitleBar, DocumentId, IconButton, LayoutViewport};

        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let button = context
            .create_component(
                document,
                IconButton::new(Icon::Sidebar, "toggle").size(ControlSize::Small),
            )
            .unwrap();
        let bar = context
            .create_component(
                document,
                AppTitleBar::new("Nana").leading(button.stable_id()),
            )
            .unwrap();
        context.append_child(bar, button).unwrap();
        context
            .layout_document(document, LayoutViewport::new(800.0, 400.0))
            .unwrap();
        context.rebuild_hit_test(document);
        (context, document, bar, button)
    }

    #[test]
    fn blank_title_bar_pointer_starts_window_drag() {
        use super::{TitleBarDragTracker, apply_title_bar_pointer, title_bar_drag_hit};

        let (context, document, bar, button) = title_bar_document();
        let bounds = context.world().layout_box(bar.stable_id()).unwrap();
        let blank_x = bounds.x + bounds.width - 24.0;
        let blank_y = bounds.y + bounds.height / 2.0;
        assert!(title_bar_drag_hit(&context, document, blank_x, blank_y));

        let button_box = context.world().layout_box(button.stable_id()).unwrap();
        assert!(!title_bar_drag_hit(
            &context,
            document,
            button_box.x + button_box.width / 2.0,
            button_box.y + button_box.height / 2.0,
        ));

        let mut state = WindowChromeState::default();
        let mut tracker = TitleBarDragTracker::default();
        let pressed = apply_title_bar_pointer(
            &mut state,
            &mut tracker,
            &context,
            document,
            &pointer_down(blank_x, blank_y),
        );
        #[cfg(target_os = "macos")]
        assert_eq!(pressed, Some(WindowChromeAction::Drag));
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(pressed, None);
            assert_eq!(
                apply_title_bar_pointer(
                    &mut state,
                    &mut tracker,
                    &context,
                    document,
                    &pointer_move(blank_x + 8.0, blank_y),
                ),
                Some(WindowChromeAction::Drag)
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn window_control_pointer_emits_chrome_action_and_skips_drag() {
        use super::{TitleBarDragTracker, apply_title_bar_pointer, title_bar_drag_hit};
        use nana_ui_runtime::{AppContext, AppTitleBar, DocumentId, LayoutViewport};

        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let bar = context
            .create_component(document, AppTitleBar::new("Nana"))
            .unwrap();
        assert!(context.assemble_app_title_bar(bar).unwrap());
        context
            .layout_document(document, LayoutViewport::new(800.0, 400.0))
            .unwrap();
        context.rebuild_hit_test(document);

        let controls = context
            .read(bar, |bar| bar.controls)
            .unwrap()
            .expect("assembled controls");
        let close = context.world().node(controls).unwrap().children[2];
        let close_box = context.world().layout_box(close).unwrap();
        let x = close_box.x + close_box.width / 2.0;
        let y = close_box.y + close_box.height / 2.0;
        assert!(!title_bar_drag_hit(&context, document, x, y));

        let mut state = WindowChromeState::default();
        let mut tracker = TitleBarDragTracker::default();
        assert_eq!(
            apply_title_bar_pointer(
                &mut state,
                &mut tracker,
                &context,
                document,
                &pointer_down(x, y),
            ),
            None
        );
        assert_eq!(
            apply_title_bar_pointer(
                &mut state,
                &mut tracker,
                &context,
                document,
                &pointer_move(x + 8.0, y),
            ),
            None
        );
        assert_eq!(
            apply_title_bar_pointer(
                &mut state,
                &mut tracker,
                &context,
                document,
                &pointer_up(x, y),
            ),
            Some(WindowChromeAction::Close)
        );
    }
}
