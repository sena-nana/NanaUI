//! Iced rendering hosted by an application-owned WGPU context.

use iced::{Color, Element, Pixels, Size, Theme, mouse};
use iced_wgpu::graphics::core::{Event, renderer, shell, time::Instant, window};
use iced_wgpu::graphics::{Antialiasing, Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::conversion;
use iced_winit::runtime::user_interface::{self, UserInterface};
use iced_winit::winit;
use iced_winit::winit::event::WindowEvent;
use iced_winit::winit::keyboard::ModifiersState;

/// Result of rendering one native UI frame into a host-provided texture.
pub struct HostedUiFrame<Message> {
    pub messages: Vec<Message>,
    pub submission: wgpu::SubmissionIndex,
}

pub(crate) struct HostedPreparedFrame<Message> {
    state: user_interface::State,
    messages: Vec<Message>,
}

impl<Message> HostedPreparedFrame<Message> {
    pub(crate) fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub(crate) fn has_layout_changed(&self) -> bool {
        self.state.has_layout_changed()
    }
}

/// Host-owned surface target for one NanaUI frame.
pub struct HostedUiTarget<'a> {
    pub window: &'a winit::window::Window,
    pub clear_color: Option<Color>,
    pub format: wgpu::TextureFormat,
    pub view: &'a wgpu::TextureView,
}

/// NanaUI renderer backed by an existing application-owned Device and Queue.
///
/// The host remains responsible for the window, surface acquisition,
/// presentation, and device-loss recovery. This type never creates an adapter
/// or requests a second device.
pub struct HostedUiRenderer<Message> {
    renderer: Renderer,
    viewport: Viewport,
    interface: Option<UserInterface<'static, Message, Theme, Renderer>>,
    events: Vec<Event>,
    cursor: mouse::Cursor,
    waker: shell::Waker,
    modifiers: ModifiersState,
    ui_dirty: bool,
    dynamic_dirty: bool,
}

impl<Message> HostedUiRenderer<Message> {
    pub fn new(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        physical_size: Size<u32>,
        window_scale_factor: f32,
        request_redraw: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        #[cfg(feature = "bundled-fonts")]
        {
            let mut font_system = iced_wgpu::graphics::text::font_system()
                .write()
                .expect("font system");
            for source in crate::ui_font_sources() {
                font_system.load_font(std::borrow::Cow::Borrowed(source));
            }
        }

        let renderer = Renderer::new(
            Engine::new(
                adapter,
                device.clone(),
                queue.clone(),
                format,
                Some(Antialiasing::MSAAx4),
                Shell::headless(),
            ),
            renderer::Settings {
                default_font: crate::ui_font(iced::font::Weight::Normal),
                default_text_size: Pixels::from(crate::UI_BASE_TEXT_SIZE),
                metrics_hinting: true,
            },
        );
        Self {
            renderer,
            viewport: viewport(physical_size, window_scale_factor),
            interface: None,
            events: Vec::new(),
            cursor: mouse::Cursor::Unavailable,
            waker: shell::Waker::new(request_redraw),
            modifiers: ModifiersState::default(),
            ui_dirty: true,
            dynamic_dirty: true,
        }
    }

    pub fn resize(&mut self, physical_size: Size<u32>, window_scale_factor: f32) {
        self.viewport = viewport(physical_size, window_scale_factor);
        self.mark_ui_dirty();
    }

    pub fn physical_size(&self) -> Size<u32> {
        self.viewport.physical_size()
    }

    pub fn logical_size(&self) -> Size {
        self.viewport.logical_size()
    }

    pub fn queue_event(&mut self, event: Event) {
        queue_event(&mut self.events, event);
    }

    pub fn set_cursor(&mut self, cursor: mouse::Cursor) {
        self.cursor = cursor;
    }

    /// Converts and queues a native window event using the renderer's retained
    /// pointer and modifier state.
    pub fn push_window_event(
        &mut self,
        event: WindowEvent,
        window: &winit::window::Window,
    ) -> bool {
        match &event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                    *position,
                    window.scale_factor() as f32,
                ));
            }
            WindowEvent::CursorLeft { .. } => self.cursor = mouse::Cursor::Unavailable,
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            _ => {}
        }

        let Some(event) =
            conversion::window_event(event, window.scale_factor() as f32, self.modifiers)
        else {
            return false;
        };
        self.queue_event(event);
        true
    }

    pub fn has_pending_events(&self) -> bool {
        !self.events.is_empty()
    }

    pub const fn is_ui_dirty(&self) -> bool {
        self.ui_dirty
    }

    pub const fn is_dynamic_dirty(&self) -> bool {
        self.dynamic_dirty
    }

    pub fn mark_ui_dirty(&mut self) {
        self.ui_dirty = true;
        self.dynamic_dirty = true;
    }

    pub fn mark_dynamic_dirty(&mut self) {
        self.dynamic_dirty = true;
    }

    /// Rebuilds the retained Iced tree and its layout.
    ///
    /// The application only calls this after a real UI state change. The
    /// widget state cache is retained, while the tree and layout are kept
    /// alive for subsequent dynamic-only frames.
    pub fn rebuild(&mut self, content: Element<'static, Message, Theme, Renderer>) {
        let cache = match self.interface.take() {
            Some(interface) => interface.into_cache(),
            None => user_interface::Cache::new(),
        };
        self.interface = Some(UserInterface::build(
            content,
            self.viewport.logical_size(),
            cache,
            &mut self.renderer,
        ));
        self.ui_dirty = false;
        self.dynamic_dirty = true;
    }

    /// Updates Iced from all queued input before the host renders the latest
    /// application state.
    pub fn update(&mut self, window: &winit::window::Window) -> Vec<Message> {
        if self.events.is_empty() {
            return Vec::new();
        }

        let events = std::mem::take(&mut self.events);
        let mut messages = Vec::new();
        let Some(interface) = self.interface.as_mut() else {
            self.events = events;
            return messages;
        };
        let _ = interface.update(
            window,
            &self.waker,
            &events,
            self.cursor,
            &mut self.renderer,
            &mut messages,
        );
        messages
    }

    pub(crate) fn prepare_frame(
        &mut self,
        window: &winit::window::Window,
        now: Instant,
    ) -> HostedPreparedFrame<Message> {
        let mut events = std::mem::take(&mut self.events);
        events.push(Event::Window(window::Event::RedrawRequested(now)));
        self.prepare_events(window, &events)
    }

    pub(crate) fn prepare_redraw(
        &mut self,
        window: &winit::window::Window,
        now: Instant,
    ) -> HostedPreparedFrame<Message> {
        self.prepare_events(
            window,
            &[Event::Window(window::Event::RedrawRequested(now))],
        )
    }

    pub(crate) fn update_prepared_redraw(
        &mut self,
        prepared: &mut HostedPreparedFrame<Message>,
        window: &winit::window::Window,
        now: Instant,
    ) {
        *prepared = self.prepare_redraw(window, now);
    }

    pub(crate) fn cache_prepared(
        &mut self,
        prepared: HostedPreparedFrame<Message>,
        window: &winit::window::Window,
    ) -> Vec<Message> {
        let HostedPreparedFrame { state, messages } = prepared;
        apply_cursor_state(window, state);
        messages
    }

    fn prepare_events(
        &mut self,
        window: &winit::window::Window,
        events: &[Event],
    ) -> HostedPreparedFrame<Message> {
        let interface = self
            .interface
            .as_mut()
            .expect("hosted UI must be rebuilt before preparing");
        let mut messages = Vec::new();
        let (state, _) = interface.update(
            window,
            &self.waker,
            events,
            self.cursor,
            &mut self.renderer,
            &mut messages,
        );
        HostedPreparedFrame { state, messages }
    }

    /// Draws the latest UI tree into the host's current surface texture.
    ///
    /// Interactive hosts must call [`Self::update`] and process its messages
    /// before this method so the same frame reflects the latest input state.
    pub fn render(
        &mut self,
        theme: &Theme,
        style: renderer::Style,
        target: HostedUiTarget<'_>,
    ) -> HostedUiFrame<Message> {
        let interface = self
            .interface
            .as_mut()
            .expect("hosted UI must be rebuilt before rendering");
        let mut messages = Vec::new();
        let (state, _) = interface.update(
            target.window,
            &self.waker,
            &[Event::Window(
                window::Event::RedrawRequested(Instant::now()),
            )],
            self.cursor,
            &mut self.renderer,
            &mut messages,
        );
        interface.draw(&mut self.renderer, theme, &style, self.cursor);
        self.dynamic_dirty = false;
        apply_cursor_state(target.window, state);
        let submission = self.renderer.present(
            target.clear_color,
            target.format,
            target.view,
            &self.viewport,
        );
        HostedUiFrame {
            messages,
            submission,
        }
    }

    pub(crate) fn present_prepared(
        &mut self,
        prepared: HostedPreparedFrame<Message>,
        theme: &Theme,
        style: renderer::Style,
        target: HostedUiTarget<'_>,
    ) -> HostedUiFrame<Message> {
        let HostedPreparedFrame { state, messages } = prepared;
        let interface = self
            .interface
            .as_mut()
            .expect("hosted UI must be rebuilt before presenting");
        interface.draw(&mut self.renderer, theme, &style, self.cursor);
        self.dynamic_dirty = false;
        apply_cursor_state(target.window, state);
        let submission = self.renderer.present(
            target.clear_color,
            target.format,
            target.view,
            &self.viewport,
        );
        HostedUiFrame {
            messages,
            submission,
        }
    }
}

fn queue_event(events: &mut Vec<Event>, event: Event) {
    if matches!(&event, Event::Mouse(mouse::Event::CursorMoved { .. }))
        && let Some(last @ Event::Mouse(mouse::Event::CursorMoved { .. })) = events.last_mut()
    {
        *last = event;
    } else {
        events.push(event);
    }
}

fn apply_cursor_state(window: &winit::window::Window, state: user_interface::State) {
    let user_interface::State::Updated {
        mouse_interaction, ..
    } = state
    else {
        return;
    };

    if let Some(icon) = conversion::mouse_interaction(mouse_interaction) {
        window.set_cursor(icon);
        window.set_cursor_visible(true);
    } else {
        window.set_cursor_visible(false);
    }
}

fn viewport(physical_size: Size<u32>, window_scale_factor: f32) -> Viewport {
    let window_scale_factor = if window_scale_factor.is_finite() && window_scale_factor > 0.0 {
        window_scale_factor
    } else {
        1.0
    };
    Viewport::with_physical_size(
        Size::new(physical_size.width.max(1), physical_size.height.max(1)),
        renderer::Scale {
            window: window_scale_factor,
            application: 1.0,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Point;

    fn cursor_moved(x: f32, y: f32) -> Event {
        Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(x, y),
        })
    }

    fn cursor_position(event: &Event) -> Option<Point> {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => Some(*position),
            _ => None,
        }
    }

    #[test]
    fn coalesces_only_adjacent_cursor_moves() {
        let mut events = Vec::new();
        queue_event(&mut events, cursor_moved(10.0, 20.0));
        queue_event(&mut events, cursor_moved(30.0, 40.0));
        queue_event(
            &mut events,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        );
        queue_event(&mut events, cursor_moved(50.0, 60.0));
        queue_event(&mut events, cursor_moved(70.0, 80.0));
        queue_event(
            &mut events,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        );
        queue_event(&mut events, cursor_moved(90.0, 100.0));
        queue_event(&mut events, cursor_moved(110.0, 120.0));

        assert_eq!(events.len(), 5);
        assert_eq!(cursor_position(&events[0]), Some(Point::new(30.0, 40.0)));
        assert!(matches!(
            events[1],
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ));
        assert_eq!(cursor_position(&events[2]), Some(Point::new(70.0, 80.0)));
        assert!(matches!(
            events[3],
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        ));
        assert_eq!(cursor_position(&events[4]), Some(Point::new(110.0, 120.0)));
    }
}
