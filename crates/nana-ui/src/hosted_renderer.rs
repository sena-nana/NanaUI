//! Iced rendering hosted by an application-owned WGPU context.

use iced::{Color, Element, Pixels, Size, Theme, mouse};
use iced_wgpu::graphics::core::{Event, renderer, shell, time::Instant, window};
use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::runtime::user_interface::{self, UserInterface};
use iced_winit::winit;

/// Result of rendering one native UI frame into a host-provided texture.
pub struct HostedUiFrame<Message> {
    pub messages: Vec<Message>,
    pub mouse_interaction: mouse::Interaction,
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
                None,
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
        self.events.push(event);
    }

    pub fn set_cursor(&mut self, cursor: mouse::Cursor) {
        self.cursor = cursor;
    }

    /// Updates and draws a UI tree into the host's current surface texture.
    pub fn render<Message>(
        &mut self,
        content: Element<'_, Message>,
        theme: &Theme,
        style: renderer::Style,
        target: HostedUiTarget<'_>,
    ) -> HostedUiFrame<Message> {
        self.events.push(Event::Window(
            window::Event::RedrawRequested(Instant::now()),
        ));
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
            &self.events,
            self.cursor,
            &mut self.renderer,
            &mut messages,
        );
        self.events.clear();
        interface.draw(&mut self.renderer, theme, &style, self.cursor);
        self.cache = interface.into_cache();
        let mouse_interaction = match state {
            user_interface::State::Updated {
                mouse_interaction, ..
            } => mouse_interaction,
            user_interface::State::Outdated => mouse::Interaction::default(),
        };
        let submission = self.renderer.present(
            target.clear_color,
            target.format,
            target.view,
            &self.viewport,
        );
        HostedUiFrame {
            messages,
            mouse_interaction,
            submission,
        }
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
