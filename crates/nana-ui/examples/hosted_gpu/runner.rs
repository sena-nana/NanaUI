use std::convert::Infallible;
use std::sync::OnceLock;
use std::time::Instant;

use iced::Element;
use iced_wgpu::wgpu;
use nana_ui::{
    HostedProgram, HostedProgramContext, HostedProgramUpdate, HostedRedraw, HostedRunError,
    HostedWindowAction, HostedWindowEvent, HostedWindowId, HostedWindowSettings, ThemeMode,
    run_hosted,
};
use nana_window::MaterialOutcome;

use crate::panel::{DemoPanel, Message};
use crate::performance::StartupProbe;
use crate::scene::SharedScene;

static STARTED_AT: OnceLock<Instant> = OnceLock::new();

pub fn run(started_at: Instant) -> Result<(), HostedRunError> {
    let _ = STARTED_AT.set(started_at);
    run_hosted::<DemoProgram>(
        HostedWindowSettings::new("NanaUI Hosted GPU Demo")
            .initial_size(1100.0, 720.0)
            .minimum_size(760.0, 520.0)
            .transparent(true),
    )
}

struct DemoProgram {
    panel: DemoPanel,
    scene: SharedScene,
    startup: StartupProbe,
}

impl HostedProgram for DemoProgram {
    type Message = Message;
    type Error = Infallible;

    fn initialize(
        context: &HostedProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let panel = DemoPanel::default();
        let colors = panel.colors();
        let scene = SharedScene::new(
            context.gpu().device(),
            context.gpu().queue(),
            context.surface_format(),
            [colors.background, colors.accent_strong],
            panel.revision(),
            context.physical_size(),
        );
        let started_at = STARTED_AT.get().copied().unwrap_or_else(Instant::now);
        Ok((
            Self {
                panel,
                scene,
                startup: StartupProbe::new(started_at),
            },
            Vec::new(),
        ))
    }

    fn update(
        &mut self,
        message: Self::Message,
        context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        let update = self.panel.update(message);
        let colors = self.panel.colors();
        self.scene.update(
            context.gpu().queue(),
            colors.background,
            colors.accent_strong,
            self.panel.revision(),
        );
        HostedProgramUpdate {
            window_action: update.window_action.map(|action| HostedWindowAction {
                id: HostedWindowId::PRIMARY,
                action,
            }),
            redraw: HostedRedraw::Primary,
            window_commands: Vec::new(),
            ui_commands: Vec::new(),
            capture_input: false,
            exit: false,
        }
    }

    fn view(&self, native_material: bool) -> Element<'static, Self::Message> {
        self.panel.view(self.scene.texture(), native_material)
    }

    fn theme_mode(&self) -> ThemeMode {
        self.panel.theme_mode()
    }

    fn window_event(
        &mut self,
        event: HostedWindowEvent,
        context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        match event {
            HostedWindowEvent::Ready { geometry, .. }
            | HostedWindowEvent::Resized { geometry, .. } => {
                self.panel.sync_maximized(geometry.maximized);
                let size = context.physical_size();
                self.scene.resize(
                    context.gpu().device(),
                    context.surface_format(),
                    size.width,
                    size.height,
                );
                HostedProgramUpdate::redraw()
            }
            HostedWindowEvent::Moved { .. }
            | HostedWindowEvent::FocusChanged { .. }
            | HostedWindowEvent::VisibilityChanged { .. }
            | HostedWindowEvent::FileHovered { .. }
            | HostedWindowEvent::FileDropped { .. }
            | HostedWindowEvent::FileHoverCancelled { .. }
            | HostedWindowEvent::KeyPressed { .. } => HostedProgramUpdate::default(),
            HostedWindowEvent::CloseRequested { .. } => HostedProgramUpdate::exit(),
        }
    }

    fn prepare_frame(&mut self, context: &HostedProgramContext<Self::Message>) {
        let mut encoder =
            context
                .gpu()
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nana-ui host scene encoder"),
                });
        self.scene.render(&mut encoder);
        context.gpu().queue().submit([encoder.finish()]);
    }

    fn rebuild_gpu(&mut self, context: &HostedProgramContext<Self::Message>) {
        let colors = self.panel.colors();
        self.scene = SharedScene::new(
            context.gpu().device(),
            context.gpu().queue(),
            context.surface_format(),
            [colors.background, colors.accent_strong],
            self.panel.revision(),
            context.physical_size(),
        );
    }

    fn frame_presented(
        &mut self,
        material: MaterialOutcome,
        _context: &HostedProgramContext<Self::Message>,
    ) -> HostedProgramUpdate {
        if self.startup.record_first_frame(material.effect) {
            HostedProgramUpdate::exit()
        } else {
            HostedProgramUpdate::default()
        }
    }
}
