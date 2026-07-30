use crate::context::HostGraphics;
use crate::panel::{DemoPanel, window_background};
use crate::performance::StartupProbe;
use crate::scene::SharedScene;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::Event;
use iced_winit::core::mouse;
use iced_winit::core::renderer;
use iced_winit::core::shell;
use iced_winit::core::time::Instant;
use iced_winit::core::window;
use iced_winit::runtime::user_interface::{self, UserInterface};
use iced_winit::winit;
use nana_window::{
    Appearance, FallbackColor, MaterialOutcome, apply_system_material, clear_system_material,
};

use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;

pub fn run(started_at: Instant) -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut Runner::Loading(Some(StartupProbe::new(started_at))))
}

enum Runner {
    Loading(Option<StartupProbe>),
    Ready(Box<Ready>),
}

struct Ready {
    window: Arc<winit::window::Window>,
    graphics: HostGraphics,
    scene: SharedScene,
    panel: DemoPanel,
    events: Vec<Event>,
    cursor: mouse::Cursor,
    cache: user_interface::Cache,
    waker: shell::Waker,
    modifiers: ModifiersState,
    resized: bool,
    startup: StartupProbe,
    material: MaterialOutcome,
}

impl ApplicationHandler for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Loading(startup) = self else {
            return;
        };
        let startup = startup
            .take()
            .expect("host startup must only initialize once");

        let window = Arc::new(
            event_loop
                .create_window(
                    winit::window::WindowAttributes::default()
                        .with_title("NanaUI Hosted GPU Demo")
                        .with_transparent(true)
                        .with_inner_size(winit::dpi::LogicalSize::new(1100.0, 720.0))
                        .with_min_inner_size(winit::dpi::LogicalSize::new(760.0, 520.0)),
                )
                .expect("host must create the demo window"),
        );
        let material = apply_system_material(
            window.as_ref(),
            Appearance::Dark,
            FallbackColor::rgba(24, 24, 24, 220),
        );
        let graphics = HostGraphics::new(window.clone());
        let panel = DemoPanel::default();
        let colors = panel.colors();
        let physical_size = graphics.viewport.physical_size();
        let scene = SharedScene::new(
            &graphics.device,
            &graphics.queue,
            graphics.format,
            [colors.background, colors.accent_strong],
            panel.revision(),
            physical_size,
        );
        let redraw_window = Arc::downgrade(&window);
        let waker = shell::Waker::new(move || {
            if let Some(window) = redraw_window.upgrade() {
                window.request_redraw();
            }
        });

        window.request_redraw();
        *self = Self::Ready(Box::new(Ready {
            window,
            graphics,
            scene,
            panel,
            events: Vec::new(),
            cursor: mouse::Cursor::Unavailable,
            cache: user_interface::Cache::new(),
            waker,
            modifiers: ModifiersState::default(),
            resized: false,
            startup,
            material,
        }));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Self::Ready(state) = self else {
            return;
        };

        match &event {
            WindowEvent::RedrawRequested => {
                state.redraw(event_loop);
                return;
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::CursorMoved { position, .. } => {
                state.cursor = mouse::Cursor::Available(conversion::cursor_position(
                    *position,
                    state.graphics.viewport.scale_factor(),
                ));
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                state.modifiers = modifiers.state();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                state.resized = true;
            }
            _ => {}
        }

        if let Some(event) =
            conversion::window_event(event, state.window.scale_factor() as f32, state.modifiers)
        {
            state.events.push(event);
            state.update_interface();
        }
    }
}

impl Ready {
    fn update_interface(&mut self) {
        let mut interface = UserInterface::build(
            self.panel
                .view(self.scene.texture(), self.material.is_native()),
            self.graphics.viewport.logical_size(),
            std::mem::take(&mut self.cache),
            &mut self.graphics.renderer,
        );
        let mut messages = Vec::new();
        let (state, _) = interface.update(
            self.window.as_ref(),
            &self.waker,
            &self.events,
            self.cursor,
            &mut self.graphics.renderer,
            &mut messages,
        );
        self.events.clear();
        self.cache = interface.into_cache();
        self.apply_cursor_state(state);

        for message in messages {
            let appearance_changed = self.panel.update(message);
            if appearance_changed {
                self.refresh_material();
            }
            let colors = self.panel.colors();
            self.scene.update(
                &self.graphics.queue,
                colors.background,
                colors.accent_strong,
                self.panel.revision(),
            );
        }
        self.window.request_redraw();
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        if self.resized {
            self.graphics.resize(&self.window);
            let size = self.graphics.viewport.physical_size();
            self.scene.resize(
                &self.graphics.device,
                self.graphics.format,
                size.width,
                size.height,
            );
            self.resized = false;
        }
        if !self.graphics.is_drawable() {
            return;
        }

        let frame = match self.graphics.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.graphics.reconfigure();
                frame
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.graphics.reconfigure();
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.graphics.recover_surface(self.window.clone());
                self.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                event_loop.exit();
                return;
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut scene_encoder =
            self.graphics
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nana-ui host scene encoder"),
                });
        self.scene.render(&mut scene_encoder);
        self.graphics.queue.submit([scene_encoder.finish()]);

        let mut interface = UserInterface::build(
            self.panel
                .view(self.scene.texture(), self.material.is_native()),
            self.graphics.viewport.logical_size(),
            std::mem::take(&mut self.cache),
            &mut self.graphics.renderer,
        );
        let (state, _) = interface.update(
            self.window.as_ref(),
            &self.waker,
            &[Event::Window(
                window::Event::RedrawRequested(Instant::now()),
            )],
            self.cursor,
            &mut self.graphics.renderer,
            &mut Vec::new(),
        );
        let colors = self.panel.colors();
        interface.draw(
            &mut self.graphics.renderer,
            &self.panel.theme(),
            &renderer::Style {
                text_color: colors.text,
            },
            self.cursor,
        );
        self.cache = interface.into_cache();
        self.apply_cursor_state(state);

        self.graphics.renderer.present(
            Some(window_background(
                colors.background,
                self.material.is_native(),
            )),
            frame.texture.format(),
            &target,
            &self.graphics.viewport,
        );
        self.graphics.queue.present(frame);
        if self.startup.record_first_frame(self.material.effect) {
            event_loop.exit();
        }
    }

    fn refresh_material(&mut self) {
        clear_system_material(self.window.as_ref());
        let (appearance, fallback) = if self.panel.is_dark() {
            (Appearance::Dark, FallbackColor::rgba(24, 24, 24, 220))
        } else {
            (Appearance::Light, FallbackColor::rgba(255, 255, 255, 232))
        };
        self.material = apply_system_material(self.window.as_ref(), appearance, fallback);
    }

    fn apply_cursor_state(&self, state: user_interface::State) {
        let user_interface::State::Updated {
            mouse_interaction, ..
        } = state
        else {
            return;
        };

        if let Some(icon) = conversion::mouse_interaction(mouse_interaction) {
            self.window.set_cursor(icon);
            self.window.set_cursor_visible(true);
        } else {
            self.window.set_cursor_visible(false);
        }
    }
}

impl Drop for Ready {
    fn drop(&mut self) {
        clear_system_material(self.window.as_ref());
    }
}
