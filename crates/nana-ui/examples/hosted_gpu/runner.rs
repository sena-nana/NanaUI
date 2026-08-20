use std::convert::Infallible;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use nana_ui::runtime::{
    Activate, Button, DocumentId, Entity, FrameworkError, GpuTextureView, List, RuntimeDocument,
    Text,
};
use nana_ui::{
    ButtonKind, HostTextureAlphaMode, HostTextureRegistry, HostedRunError, RuntimeProgram,
    RuntimeProgramContext, RuntimeProgramUpdate, RuntimeWindowSettings, ThemeMode, run_runtime,
};
use nana_ui_platform::{WindowEvent, WindowId};

use crate::panel::{DemoPanel, Message};
use crate::performance::StartupProbe;
use crate::scene::SharedScene;

const PREVIEW_SLOT: &str = "preview";
const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

static STARTED_AT: OnceLock<Instant> = OnceLock::new();

pub fn run(started_at: Instant) -> Result<(), HostedRunError> {
    let _ = STARTED_AT.set(started_at);
    let mut settings = RuntimeWindowSettings::new("NanaUI Hosted GPU Demo")
        .initial_size(1100.0, 720.0)
        .minimum_size(760.0, 520.0)
        .system_caption(true);
    settings.transparent = true;
    run_runtime::<DemoProgram>(settings)
}

struct DemoProgram {
    panel: DemoPanel,
    scene: SharedScene,
    document: RuntimeDocument,
    preview: Entity<GpuTextureView>,
    version: Entity<Text>,
    theme_button: Entity<Button>,
    textures: HostTextureRegistry,
    refresh: Arc<AtomicBool>,
    toggle_theme: Arc<AtomicBool>,
    startup: StartupProbe,
}

impl DemoProgram {
    fn mount(
        context: &RuntimeProgramContext<Message>,
        panel: DemoPanel,
        scene: SharedScene,
        startup: StartupProbe,
    ) -> Result<Self, FrameworkError> {
        let refresh = Arc::new(AtomicBool::new(false));
        let toggle_theme = Arc::new(AtomicBool::new(false));
        let document_id = DocumentId::new(1).expect("hosted gpu document");
        let mut document = RuntimeDocument::new(document_id);
        let root = document
            .context_mut()
            .create_component(document_id, List::new().label("Hosted GPU"))?;
        let title = document
            .context_mut()
            .create_component(document_id, Text::new("NANA 实时预览"))?;
        let theme_button = document.context_mut().create_component(
            document_id,
            Button::new(panel.theme_label()).kind(ButtonKind::Text),
        )?;
        let preview = document
            .context_mut()
            .create_component(document_id, GpuTextureView::new(PREVIEW_SLOT))?;
        let version = document
            .context_mut()
            .create_component(document_id, Text::new(panel.version_label()))?;
        let refresh_button = document.context_mut().create_component(
            document_id,
            Button::new("刷新预览").kind(ButtonKind::Primary),
        )?;
        document.context_mut().append_child(root, title)?;
        document.context_mut().append_child(root, theme_button)?;
        document.context_mut().append_child(root, preview)?;
        document.context_mut().append_child(root, version)?;
        document.context_mut().append_child(root, refresh_button)?;

        let pending_refresh = Arc::clone(&refresh);
        document
            .context_mut()
            .on(refresh_button, move |_button, _event: &Activate, _cx| {
                pending_refresh.store(true, Ordering::SeqCst);
            })?;
        let pending_theme = Arc::clone(&toggle_theme);
        document
            .context_mut()
            .on(theme_button, move |_button, _event: &Activate, _cx| {
                pending_theme.store(true, Ordering::SeqCst);
            })?;

        let textures = HostTextureRegistry::new();
        let (width, height) = scene.size();
        textures.register(
            PREVIEW_SLOT,
            scene.texture(),
            width,
            height,
            HostTextureAlphaMode::Opaque,
        );
        let _ = context;
        Ok(Self {
            panel,
            scene,
            document,
            preview,
            version,
            theme_button,
            textures,
            refresh,
            toggle_theme,
            startup,
        })
    }

    fn apply(&mut self, message: Message, context: &RuntimeProgramContext<Message>) {
        self.panel.update(message);
        let colors = self.panel.colors();
        self.scene.update(
            context.gpu().queue(),
            colors.background,
            colors.accent_strong,
            self.panel.revision(),
        );
        let _ = self
            .document
            .context_mut()
            .update_component(self.version, |text, _| {
                text.value = self.panel.version_label();
            });
        let _ = self
            .document
            .context_mut()
            .update_component(self.theme_button, |button, _| {
                button.label = self.panel.theme_label().to_owned();
            });
        let generation = self.scene.texture().generation();
        let _ = self
            .document
            .context_mut()
            .update_component(self.preview, |view, _| {
                view.replace_view(generation);
                view.invalidate_content();
            });
    }

    fn register_texture(&self) {
        let (width, height) = self.scene.size();
        self.textures.register(
            PREVIEW_SLOT,
            self.scene.texture(),
            width,
            height,
            HostTextureAlphaMode::Opaque,
        );
    }
}

impl RuntimeProgram for DemoProgram {
    type Message = Message;
    type Error = Infallible;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let panel = DemoPanel::default();
        let colors = panel.colors();
        let size = context.geometry().physical_size;
        let scene = SharedScene::new(
            context.gpu().device(),
            context.gpu().queue(),
            SURFACE_FORMAT,
            [colors.background, colors.accent_strong],
            panel.revision(),
            size,
        );
        let started_at = STARTED_AT.get().copied().unwrap_or_else(Instant::now);
        let program = Self::mount(context, panel, scene, StartupProbe::new(started_at))
            .expect("hosted gpu document");
        Ok((program, Vec::new()))
    }

    fn document(&self, id: WindowId) -> Option<&RuntimeDocument> {
        (id == WindowId::PRIMARY).then_some(&self.document)
    }

    fn document_mut(&mut self, id: WindowId) -> Option<&mut RuntimeDocument> {
        (id == WindowId::PRIMARY).then_some(&mut self.document)
    }

    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.apply(message, context);
        RuntimeProgramUpdate::redraw_all()
    }

    fn theme_mode(&self) -> ThemeMode {
        self.panel.theme_mode()
    }

    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        Some(self.textures.clone())
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::Ready { geometry, .. } | WindowEvent::Resized { geometry, .. } => {
                self.scene.resize(
                    context.gpu().device(),
                    SURFACE_FORMAT,
                    geometry.physical_size.0,
                    geometry.physical_size.1,
                );
                self.register_texture();
                let generation = self.scene.texture().generation();
                let _ = self
                    .document
                    .context_mut()
                    .update_component(self.preview, |view, _| {
                        view.replace_view(generation);
                    });
                RuntimeProgramUpdate::redraw_all()
            }
            WindowEvent::CloseRequested { .. } => RuntimeProgramUpdate::exit(),
            _ => RuntimeProgramUpdate::default(),
        }
    }

    fn prepare_window_frame(
        &mut self,
        _id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        let mut encoder =
            context
                .gpu()
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nana-ui host scene encoder"),
                });
        self.scene.render(&mut encoder);
        context.gpu().queue().submit([encoder.finish()]);
        self.scene.texture().invalidate();
        self.register_texture();
    }

    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        let colors = self.panel.colors();
        self.scene = SharedScene::new(
            context.gpu().device(),
            context.gpu().queue(),
            SURFACE_FORMAT,
            [colors.background, colors.accent_strong],
            self.panel.revision(),
            context.geometry().physical_size,
        );
        self.register_texture();
    }

    fn input_event(
        &mut self,
        id: WindowId,
        _event: &nana_ui_platform::InputEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let mut changed = false;
        if self.refresh.swap(false, Ordering::SeqCst) {
            self.apply(Message::Refresh, context);
            changed = true;
        }
        if self.toggle_theme.swap(false, Ordering::SeqCst) {
            self.apply(Message::ToggleTheme, context);
            changed = true;
        }
        Ok(if changed {
            RuntimeProgramUpdate::redraw_all()
        } else {
            RuntimeProgramUpdate::redraw(id)
        })
    }

    fn window_frame_presented(
        &mut self,
        _id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        if self.startup.record_first_frame(context.material()) {
            RuntimeProgramUpdate::exit()
        } else {
            RuntimeProgramUpdate::default()
        }
    }
}
