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
pub struct HostedUiRenderer {
    renderer: Renderer,
    viewport: Viewport,
    cache: user_interface::Cache,
    events: Vec<Event>,
    cursor: mouse::Cursor,
    waker: shell::Waker,
    modifiers: ModifiersState,
}

impl HostedUiRenderer {
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
        let viewport = viewport(physical_size, window_scale_factor);
        Self {
            renderer,
            viewport,
            cache: user_interface::Cache::new(),
            events: Vec::new(),
            cursor: mouse::Cursor::Unavailable,
            waker: shell::Waker::new(request_redraw),
            modifiers: ModifiersState::default(),
        }
    }

    pub fn resize(&mut self, physical_size: Size<u32>, window_scale_factor: f32) {
        self.viewport = viewport(physical_size, window_scale_factor);
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

    /// Updates Iced from all queued input before the host renders the latest
    /// application state.
    pub fn update<Message>(
        &mut self,
        content: Element<'_, Message>,
        window: &winit::window::Window,
    ) -> Vec<Message> {
        if self.events.is_empty() {
            return Vec::new();
        }

        let mut interface = UserInterface::build(
            content,
            self.viewport.logical_size(),
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            window,
            &self.waker,
            &self.events,
            self.cursor,
            &mut self.renderer,
            &mut messages,
        );
        self.events.clear();
        self.cache = interface.into_cache();
        messages
    }

    /// Draws the latest UI tree into the host's current surface texture.
    ///
    /// Interactive hosts must call [`Self::update`] and process its messages
    /// before this method so the same frame reflects the latest input state.
    pub fn render<Message>(
        &mut self,
        content: Element<'_, Message>,
        theme: &Theme,
        style: renderer::Style,
        target: HostedUiTarget<'_>,
    ) -> HostedUiFrame<Message> {
        let mut interface = UserInterface::build(
            content,
            self.viewport.logical_size(),
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
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
        self.cache = interface.into_cache();
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
