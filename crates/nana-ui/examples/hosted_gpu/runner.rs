use std::convert::Infallible;
use std::sync::{Arc, Mutex, OnceLock};
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

struct PreviewProducer(Arc<Mutex<SharedScene>>);

impl std::fmt::Debug for PreviewProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewProducer")
    }
}

impl nana_ui::SceneResourceProducer for PreviewProducer {
    fn encode(
        &self,
        _: &nana_ui::runtime::CustomRenderNode,
        context: nana_ui::SceneResourceEncodeContext<'_>,
    ) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|error| error.to_string())?
            .render(context.encoder);
        Ok(())
    }
}

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
    scene: Arc<Mutex<SharedScene>>,
    document: RuntimeDocument,
    version: Entity<Text>,
    theme_button: Entity<Button>,
    textures: HostTextureRegistry,
    producers: nana_ui::SceneResourceProducerRegistry,
    startup: StartupProbe,
}

impl DemoProgram {
    fn mount(
        context: &RuntimeProgramContext<Message>,
        panel: DemoPanel,
        scene: SharedScene,
        startup: StartupProbe,
    ) -> Result<Self, FrameworkError> {
        let document_id = DocumentId::new(1).expect("hosted gpu document");
        let mut document = RuntimeDocument::new(document_id);
        let (version, theme_button) = document.context_mut().build(document_id, |ui| {
            ui.with("root", List::new().label("Hosted GPU"), |ui| {
                ui.child("title", Text::new("NANA 实时预览"));
                let theme_button = ui.child(
                    "theme",
                    Button::new(panel.theme_label()).kind(ButtonKind::Text),
                );
                ui.child("preview", GpuTextureView::new(PREVIEW_SLOT));
                let version = ui.child("version", Text::new(panel.version_label()));
                let refresh =
                    ui.child("refresh", Button::new("刷新预览").kind(ButtonKind::Primary));
                ui.on(refresh, move |_button, _event: &Activate, cx| {
                    cx.dispatch_program(Message::Refresh);
                });
                ui.on(theme_button, move |_button, _event: &Activate, cx| {
                    cx.dispatch_program(Message::ToggleTheme);
                });
                (version, theme_button)
            })
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
        let scene = Arc::new(Mutex::new(scene));
        let mut producers = nana_ui::SceneResourceProducerRegistry::new();
        producers.insert(PREVIEW_SLOT, Arc::new(PreviewProducer(Arc::clone(&scene))));
        Ok(Self {
            panel,
            scene,
            document,
            version,
            theme_button,
            textures,
            producers,
            startup,
        })
    }

    fn apply(&mut self, message: Message, context: &RuntimeProgramContext<Message>) {
        self.panel.update(message);
        let colors = self.panel.colors();
        self.scene.lock().expect("preview scene").update(
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
        let _ = self.textures.slot(PREVIEW_SLOT).invalidate();
    }

    fn register_texture(&self) {
        let scene = self.scene.lock().expect("preview scene");
        let (width, height) = scene.size();
        self.textures.register(
            PREVIEW_SLOT,
            scene.texture(),
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

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        let document = { (id == WindowId::PRIMARY).then_some(&self.document) };
        Ok(document.map(f))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        let document = { (id == WindowId::PRIMARY).then_some(&mut self.document) };
        Ok(document.map(f))
    }

    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.apply(message, context);
        RuntimeProgramUpdate::redraw_all()
    }

    fn frame_demand(&self, _id: WindowId) -> nana_ui::FrameDemand {
        self.startup.demand()
    }

    fn theme_mode(&self) -> ThemeMode {
        self.panel.theme_mode()
    }

    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        Some(self.textures.clone())
    }

    fn scene_resource_producers(
        &self,
        _id: WindowId,
    ) -> Option<nana_ui::SceneResourceProducerRegistry> {
        Some(self.producers.clone())
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::Ready { geometry, .. } | WindowEvent::Resized { geometry, .. } => {
                self.scene.lock().expect("preview scene").resize(
                    context.gpu().device(),
                    SURFACE_FORMAT,
                    geometry.physical_size.0,
                    geometry.physical_size.1,
                );
                self.register_texture();
                RuntimeProgramUpdate::redraw_all()
            }
            WindowEvent::CloseRequested { .. } => RuntimeProgramUpdate::exit(),
            _ => RuntimeProgramUpdate::default(),
        }
    }

    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        let colors = self.panel.colors();
        *self.scene.lock().expect("preview scene") = SharedScene::new(
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
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        Ok(RuntimeProgramUpdate::redraw(id))
    }

    fn window_frame_presented(
        &mut self,
        _id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        if self.startup.record_frame(context) {
            RuntimeProgramUpdate::exit()
        } else {
            RuntimeProgramUpdate::default()
        }
    }
}
